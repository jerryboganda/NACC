//! SQLite persistence and versioned migrations for durable NACC state
//! (master plan S4.4, S16 -- Phase 2 scope for real schema and migrations).
//!
//! One local SQLite database per NACC user profile (S4.4), opened once at
//! startup via [`Database::open`]. This crate implements real schema,
//! migrations, and repositories for exactly the data groups Phase 2's own
//! deliverable list names: settings, role profiles, events, and audit
//! records. S4.4 lists many more data groups (provider installations,
//! discovered models, workflow templates/runs, worktree allocations,
//! quality-gate results, review findings, CI/CD records, approvals, policy
//! decisions, usage estimates, updater state); those are added as their
//! owning phase's crate gains real logic to back them (each such crate's
//! own doc comment already names its target phase) rather than
//! speculatively schema'd now against a design those phases have not made
//! yet -- exactly the scope discipline Phase 1 applied to every other
//! placeholder crate, applied here to storage too. Migrations are additive
//! and versioned for this reason: later phases add new migration versions,
//! they do not require rewriting this one.
//!
//! # Concurrency model
//!
//! NACC is a single-process, single-user desktop app, not a
//! multi-client server -- there is exactly one [`Database`] per running
//! app, holding one `rusqlite::Connection` behind an `Arc<Mutex<_>>`.
//! Every repository method that touches the connection is `async` and
//! internally runs the actual (synchronous) SQLite call inside
//! `tokio::task::spawn_blocking`, so a query never blocks the async
//! runtime's worker threads -- the standard, documented pattern for
//! embedding a sync SQLite binding in an async app (see this crate's own
//! Cargo.toml comment in the workspace root for why `rusqlite` over
//! `sqlx`). [`Database::open`] itself, and reading the schema version, stay
//! plain synchronous functions: both are startup-only or metadata-only, not
//! on any hot path, matching `nacc_observability::init_tracing`'s existing
//! precedent of a synchronous one-time startup call.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::Connection;

mod audit;
mod events;
mod migrations;
mod role_profiles;
mod settings;

/// Errors from any storage operation. Repository methods that can fail for
/// entity-specific reasons (e.g. "no role profile with that id") add a
/// dedicated variant here rather than overloading `Sqlite`/`Serialization`
/// with a stringly-typed message, matching every other typed-error crate in
/// this workspace.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration error: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("failed to (de)serialize a stored value as JSON: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("stored {entity} value {value:?} is not valid: {detail}")]
    CorruptStoredValue {
        entity: &'static str,
        value: String,
        detail: String,
    },
    #[error("no role profile found with id {0}")]
    RoleProfileNotFound(nacc_domain::RoleProfileId),
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// A handle to NACC's one local SQLite database. Cheap to `Clone` (an
/// `Arc` clone) -- hold one in `AppState` and pass clones to whatever needs
/// storage access, rather than passing `&Database` references around.
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// Open (creating if absent) the SQLite database at `path`, applying
    /// every pending migration transactionally before returning. Creates
    /// `path`'s parent directory if it does not exist yet (the app-data
    /// directory on a fresh install).
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path)?;
        Self::configure_and_migrate(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// An in-memory database with the same schema, for tests -- never used
    /// by the running app itself, which always persists to disk.
    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        Self::configure_and_migrate(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn configure_and_migrate(conn: &mut Connection) -> Result<()> {
        // WAL: readers do not block the writer and vice versa -- the
        // right default for a desktop app whose UI keeps reading state
        // (events, run history) while a background task keeps writing it.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // Foreign keys are opt-in per SQLite connection; the schema below
        // declares real foreign-key relationships, so this must be on.
        conn.pragma_update(None, "foreign_keys", true)?;
        let migrations = migrations::migrations();
        let before = migrations.current_version(conn)?;
        migrations.to_latest(conn)?;
        let after = migrations.current_version(conn)?;
        // Logged here, once, at the single place migrations actually run
        // -- not left for every caller of `open`/`open_in_memory` to
        // remember to log for themselves (master plan S22: structured
        // logs). `src-tauri`'s startup log line is a separate, higher-
        // level "the app is up" event; this one is "the schema changed
        // (or didn't)," which matters even to a caller that isn't
        // `src-tauri` -- a future CLI diagnostics tool, for instance.
        tracing::debug!(?before, ?after, "database opened and migrated");
        Ok(())
    }

    /// The applied schema version, mainly for diagnostics (see
    /// `src-tauri`'s `get_app_diagnostics` command, which reports this).
    pub fn schema_version(&self) -> Result<rusqlite_migration::SchemaVersion> {
        let conn = self.lock();
        Ok(migrations::migrations().current_version(&conn)?)
    }

    /// Write a transactionally-consistent snapshot of the whole database
    /// to `dest`, using SQLite's own `VACUUM INTO` rather than a raw file
    /// copy. That distinction matters under WAL mode specifically: a plain
    /// file copy of an open WAL-mode database can capture the main file
    /// mid-write, with recent changes still sitting in the separate `-wal`
    /// file rather than checkpointed in -- `VACUUM INTO` always produces a
    /// complete, self-contained, valid database file, safe to take while
    /// the app keeps reading and writing. Master plan S4.4's
    /// "backup/restore tests" bullet; also the primitive Phase 12's
    /// diagnostics-bundle export (S17.15) will reuse.
    pub async fn backup_to(&self, dest: &Path) -> Result<()> {
        let conn = Arc::clone(&self.conn);
        let dest_str = dest.to_string_lossy().into_owned();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            conn.execute("VACUUM INTO ?1", [dest_str])?;
            Ok(())
        })
        .await
        .expect("storage worker thread panicked")
    }

    /// Restore a database previously written by [`Self::backup_to`]: copy
    /// `backup_path` to `target_path` and open it there. `target_path`
    /// must not already be in use by a live `Database` -- this is a
    /// restore-to-a-fresh-location operation, not an in-place swap of an
    /// already-open connection (SQLite, and Windows file locking, do not
    /// make replacing an open database file underneath itself safe).
    /// Opening the restored copy re-runs migrations, so restoring a backup
    /// taken at an older schema version upgrades it in place -- the same
    /// mechanism [`Self::open`] always uses, exercised here on real
    /// restored data rather than a fresh database.
    pub async fn restore_from(backup_path: &Path, target_path: &Path) -> Result<Database> {
        let backup_path = backup_path.to_owned();
        let target_path_owned = target_path.to_owned();
        tokio::task::spawn_blocking(move || -> Result<()> {
            if let Some(parent) = target_path_owned.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&backup_path, &target_path_owned)?;
            Ok(())
        })
        .await
        .expect("storage worker thread panicked")?;
        Database::open(target_path)
    }

    fn connection(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        lock(&self.conn)
    }
}

/// A poisoned mutex means some other blocking task panicked while holding
/// the lock -- a bug there, not a reason to poison every future storage
/// call for the process's remaining lifetime (matches the
/// `panic = "unwind"` + process-boundary-catches-panics philosophy already
/// stated in the workspace root `Cargo.toml`'s `[profile.release]`
/// comment).
fn lock(conn: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "nacc-storage-test-{label}-{}",
            nacc_domain::ProjectId::new()
        ))
    }

    #[test]
    fn open_creates_parent_directory_and_reports_latest_schema_version() {
        let path = temp_db_path("open").join("nested").join("nacc.sqlite");
        assert!(!path.exists());

        let db = Database::open(&path).expect("open should create the directory and the file");
        assert!(path.exists(), "database file must be created");
        assert!(
            matches!(
                db.schema_version().unwrap(),
                rusqlite_migration::SchemaVersion::Inside(_)
            ),
            "a freshly opened database must be at a real, non-empty schema version"
        );

        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[tokio::test]
    async fn backup_and_restore_round_trips_stored_data() {
        let dir = temp_db_path("backup-restore");
        let db_path = dir.join("nacc.sqlite");
        let db = Database::open(&db_path).unwrap();
        db.set_setting("theme", "dark").await.unwrap();

        let backup_path = dir.join("nacc-backup.sqlite");
        db.backup_to(&backup_path)
            .await
            .expect("backup_to should produce a standalone snapshot file");
        assert!(backup_path.exists());

        let restore_path = dir.join("nacc-restored.sqlite");
        let restored = Database::restore_from(&backup_path, &restore_path)
            .await
            .expect("restore_from should copy and reopen the backup");
        let value = restored.get_setting("theme").await.unwrap();
        assert_eq!(
            value.as_deref(),
            Some("dark"),
            "data written before the backup must survive backup + restore"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

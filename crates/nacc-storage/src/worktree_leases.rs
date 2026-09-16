//! Worktree lease persistence (master plan S16/S17 -- the durable half of
//! the worktree lifecycle `nacc-worktree` implements).
//!
//! The lease record is what makes startup reconciliation possible at all:
//! `git worktree list` can say which worktrees exist, but not which run
//! owns one, which commit it was branched from, whether its owner is still
//! alive, or whether it was already quarantined. Those facts live here.
//!
//! Enum-typed columns follow this crate's established convention
//! (`migrations.rs`'s module doc): the domain type's own `serde_json`
//! encoding, in a `..._json` column, read back with `serde_json::from_str`
//! -- one source of truth for wire and storage, never a second hand-written
//! mapping that could drift.

use nacc_domain::{ProjectId, WorkflowRunId, WorktreeLease, WorktreeLeaseId, WorktreeState};
use rusqlite::{OptionalExtension, Row};

use crate::{lock, Database, Result, StorageError};

fn row_to_lease(
    row: &Row<'_>,
) -> rusqlite::Result<std::result::Result<WorktreeLease, StorageError>> {
    let state_json: String = row.get("state_json")?;
    let project_id: String = row.get("project_id")?;
    let workflow_run_id: Option<String> = row.get("workflow_run_id")?;
    let node_run_id: Option<String> = row.get("node_run_id")?;
    let id: String = row.get("id")?;

    let parse = || -> std::result::Result<WorktreeLease, StorageError> {
        Ok(WorktreeLease {
            id: id.parse().map_err(|e| StorageError::CorruptStoredValue {
                entity: "worktree_leases.id",
                value: id.clone(),
                detail: format!("{e}"),
            })?,
            project_id: parse_uuid::<ProjectId>(&project_id, "worktree_leases.project_id")?,
            workflow_run_id: match workflow_run_id {
                Some(raw) => Some(parse_uuid::<WorkflowRunId>(
                    &raw,
                    "worktree_leases.workflow_run_id",
                )?),
                None => None,
            },
            node_run_id: match node_run_id {
                Some(raw) => Some(parse_uuid::<nacc_domain::NodeRunId>(
                    &raw,
                    "worktree_leases.node_run_id",
                )?),
                None => None,
            },
            path: row_get(row, "path")?,
            branch: row_get(row, "branch")?,
            base_commit: row_get(row, "base_commit")?,
            head_commit: row_get(row, "head_commit")?,
            state: serde_json::from_str(&state_json)?,
            owner_process_id: row_get(row, "owner_process_id")?,
            quarantine_reason: row_get(row, "quarantine_reason")?,
            created_at_millis: row_get_millis(row, "created_at_millis")?,
            updated_at_millis: row_get_millis(row, "updated_at_millis")?,
        })
    };
    Ok(parse())
}

/// `row.get` returns a `rusqlite::Result`; inside the closure above the
/// error type has to unify with `StorageError` (`rusqlite::Error` converts
/// into `StorageError::Sqlite` via `#[from]`), so this helper does the
/// conversion explicitly rather than relying on `?`'s inference in a
/// context where two error types are in play.
fn row_get<T: rusqlite::types::FromSql>(
    row: &Row<'_>,
    column: &str,
) -> std::result::Result<T, StorageError> {
    Ok(row.get(column)?)
}

/// Timestamps are stored as SQLite `INTEGER` (i64) and read back as `u64`
/// -- rusqlite has no `FromSql` impl for `u64` at all, so the conversion
/// is explicit here rather than papered over with a cast at each call site.
fn row_get_millis(row: &Row<'_>, column: &str) -> std::result::Result<u64, StorageError> {
    let raw: i64 = row.get(column)?;
    Ok(raw.max(0) as u64)
}

fn parse_uuid<T: std::str::FromStr<Err = nacc_domain::DomainError>>(
    raw: &str,
    entity: &'static str,
) -> std::result::Result<T, StorageError> {
    raw.parse().map_err(
        |e: nacc_domain::DomainError| StorageError::CorruptStoredValue {
            entity,
            value: raw.to_string(),
            detail: format!("{e}"),
        },
    )
}

impl Database {
    /// Insert a new lease. Fails if the id already exists (a lease id is
    /// generated once, by `nacc-worktree`), which is the correct
    /// conservative behavior: silently overwriting an existing lease would
    /// lose the very record reconciliation depends on.
    pub async fn insert_worktree_lease(&self, lease: &WorktreeLease) -> Result<()> {
        let conn = self.connection();
        let lease = lease.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            let state_json = serde_json::to_string(&lease.state)?;
            conn.execute(
                "INSERT INTO worktree_leases (
                    id, project_id, workflow_run_id, node_run_id, path, branch,
                    base_commit, head_commit, state_json, owner_process_id,
                    quarantine_reason, created_at_millis, updated_at_millis
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                rusqlite::params![
                    lease.id.to_string(),
                    lease.project_id.to_string(),
                    lease.workflow_run_id.map(|id| id.to_string()),
                    lease.node_run_id.map(|id| id.to_string()),
                    lease.path,
                    lease.branch,
                    lease.base_commit,
                    lease.head_commit,
                    state_json,
                    lease.owner_process_id,
                    lease.quarantine_reason,
                    lease.created_at_millis as i64,
                    lease.updated_at_millis as i64,
                ],
            )?;
            Ok(())
        })
        .await
        .expect("storage worker thread panicked")
    }

    /// Persist every field of an existing lease. Used by drift detection,
    /// release, and quarantine, so all three go through one write path
    /// instead of three subtly different UPDATE statements.
    pub async fn update_worktree_lease(&self, lease: &WorktreeLease) -> Result<()> {
        let conn = self.connection();
        let lease = lease.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            let state_json = serde_json::to_string(&lease.state)?;
            let changed = conn.execute(
                "UPDATE worktree_leases SET
                    project_id = ?2, workflow_run_id = ?3, node_run_id = ?4, path = ?5,
                    branch = ?6, base_commit = ?7, head_commit = ?8, state_json = ?9,
                    owner_process_id = ?10, quarantine_reason = ?11,
                    updated_at_millis = ?12
                 WHERE id = ?1",
                rusqlite::params![
                    lease.id.to_string(),
                    lease.project_id.to_string(),
                    lease.workflow_run_id.map(|id| id.to_string()),
                    lease.node_run_id.map(|id| id.to_string()),
                    lease.path,
                    lease.branch,
                    lease.base_commit,
                    lease.head_commit,
                    state_json,
                    lease.owner_process_id,
                    lease.quarantine_reason,
                    lease.updated_at_millis as i64,
                ],
            )?;
            if changed == 0 {
                return Err(StorageError::WorktreeLeaseNotFound(lease.id));
            }
            Ok(())
        })
        .await
        .expect("storage worker thread panicked")
    }

    pub async fn get_worktree_lease(&self, id: WorktreeLeaseId) -> Result<Option<WorktreeLease>> {
        let conn = self.connection();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<WorktreeLease>> {
            let conn = lock(&conn);
            let mut stmt = conn.prepare(
                "SELECT id, project_id, workflow_run_id, node_run_id, path, branch,
                        base_commit, head_commit, state_json, owner_process_id,
                        quarantine_reason, created_at_millis, updated_at_millis
                 FROM worktree_leases WHERE id = ?1",
            )?;
            // Bound to a local before returning, deliberately: a tail-position
            // `?` temporary would outlive `stmt` and fail to compile (the
            // same E0597 this workspace hit in Phase 2's list_* functions).
            let found = stmt.query_row([&id], row_to_lease).optional()?;
            match found {
                Some(Ok(lease)) => Ok(Some(lease)),
                Some(Err(err)) => Err(err),
                None => Ok(None),
            }
        })
        .await
        .expect("storage worker thread panicked")
    }

    /// Every lease, newest first.
    pub async fn list_worktree_leases(&self) -> Result<Vec<WorktreeLease>> {
        self.list_leases_where("", None).await
    }

    /// Leases belonging to one project, newest first.
    pub async fn list_worktree_leases_for_project(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<WorktreeLease>> {
        self.list_leases_where("WHERE project_id = ?1", Some(project_id.to_string()))
            .await
    }

    /// Active leases across every project -- the set startup reconciliation
    /// walks (master plan S16/S17.15).
    pub async fn list_active_worktree_leases(&self) -> Result<Vec<WorktreeLease>> {
        let state_json = serde_json::to_string(&WorktreeState::Active)?;
        self.list_leases_where("WHERE state_json = ?1", Some(state_json))
            .await
    }

    async fn list_leases_where(
        &self,
        clause: &'static str,
        param: Option<String>,
    ) -> Result<Vec<WorktreeLease>> {
        let conn = self.connection();
        let sql = format!(
            "SELECT id, project_id, workflow_run_id, node_run_id, path, branch,
                    base_commit, head_commit, state_json, owner_process_id,
                    quarantine_reason, created_at_millis, updated_at_millis
             FROM worktree_leases {clause} ORDER BY created_at_millis DESC, rowid DESC"
        );
        tokio::task::spawn_blocking(move || -> Result<Vec<WorktreeLease>> {
            let conn = lock(&conn);
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = match param {
                Some(param) => stmt.query([param])?,
                None => stmt.query([])?,
            };
            let mut leases = Vec::new();
            while let Some(row) = rows.next()? {
                leases.push(row_to_lease(row)??);
            }
            Ok(leases)
        })
        .await
        .expect("storage worker thread panicked")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn lease(label: &str) -> WorktreeLease {
        WorktreeLease {
            id: WorktreeLeaseId::new(),
            project_id: ProjectId::new(),
            workflow_run_id: Some(WorkflowRunId::new()),
            node_run_id: None,
            path: format!("C:\\nacc-worktrees\\{label}"),
            branch: format!("nacc/{label}"),
            base_commit: "a".repeat(40),
            head_commit: None,
            state: WorktreeState::Active,
            owner_process_id: Some(4321),
            quarantine_reason: None,
            created_at_millis: 1_735_000_000_000,
            updated_at_millis: 1_735_000_000_000,
        }
    }

    #[tokio::test]
    async fn a_lease_round_trips_with_every_field_intact() {
        let db = Database::open_in_memory().unwrap();
        let original = lease("impl-1");
        db.insert_worktree_lease(&original).await.unwrap();

        let loaded = db
            .get_worktree_lease(original.id)
            .await
            .unwrap()
            .expect("the lease must be found by its id");
        assert_eq!(loaded.path, original.path);
        assert_eq!(loaded.branch, original.branch);
        assert_eq!(loaded.state, WorktreeState::Active);
        assert_eq!(loaded.owner_process_id, Some(4321));
        assert_eq!(loaded.workflow_run_id, original.workflow_run_id);
    }

    #[tokio::test]
    async fn an_unknown_lease_id_reads_as_none_not_an_error() {
        let db = Database::open_in_memory().unwrap();
        assert!(db
            .get_worktree_lease(WorktreeLeaseId::new())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn inserting_the_same_lease_id_twice_is_a_real_error() {
        let db = Database::open_in_memory().unwrap();
        let original = lease("dup");
        db.insert_worktree_lease(&original).await.unwrap();
        assert!(db.insert_worktree_lease(&original).await.is_err());
    }

    #[tokio::test]
    async fn updating_an_unknown_lease_reports_not_found() {
        let db = Database::open_in_memory().unwrap();
        let err = db.update_worktree_lease(&lease("ghost")).await.unwrap_err();
        assert!(matches!(err, StorageError::WorktreeLeaseNotFound(_)));
    }

    #[tokio::test]
    async fn quarantine_state_and_reason_persist() {
        let db = Database::open_in_memory().unwrap();
        let mut original = lease("dirty");
        db.insert_worktree_lease(&original).await.unwrap();

        original.state = WorktreeState::Quarantined;
        original.quarantine_reason = Some("uncommitted changes at release".into());
        original.updated_at_millis = original.created_at_millis + 5;
        db.update_worktree_lease(&original).await.unwrap();

        let loaded = db.get_worktree_lease(original.id).await.unwrap().unwrap();
        assert_eq!(loaded.state, WorktreeState::Quarantined);
        assert_eq!(
            loaded.quarantine_reason.as_deref(),
            Some("uncommitted changes at release")
        );

        // And it must drop out of the active set, which is what
        // reconciliation filters on.
        assert!(db.list_active_worktree_leases().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn listing_scopes_by_project_and_by_active_state() {
        let db = Database::open_in_memory().unwrap();
        let project = ProjectId::new();
        let mut a = lease("a");
        a.project_id = project;
        let mut b = lease("b");
        b.project_id = project;
        b.state = WorktreeState::Released;
        let c = lease("c"); // a different project
        for l in [&a, &b, &c] {
            db.insert_worktree_lease(l).await.unwrap();
        }

        let for_project = db.list_worktree_leases_for_project(project).await.unwrap();
        assert_eq!(for_project.len(), 2);
        assert!(for_project.iter().all(|l| l.project_id == project));

        let active = db.list_active_worktree_leases().await.unwrap();
        assert_eq!(active.len(), 2, "b is released, so only a and c are active");
        assert!(active.iter().all(|l| l.state == WorktreeState::Active));
    }
}

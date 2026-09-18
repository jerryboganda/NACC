//! Provider installation and capability-snapshot persistence (master plan
//! S4.4's "provider installations, discovered models" data group, S8.3's
//! versioned/timestamped capability record).
//!
//! # Why this crate depends on `nacc-provider-core`
//!
//! Every other repository in this workspace stores *domain* types, and
//! `nacc-provider-core` holds the probe/snapshot types (it is where the
//! adapter contract lives, so it is where "what a provider reported" is
//! defined). The dependency is one-directional and acyclic --
//! `nacc-provider-core` never mentions storage -- and the alternative
//! (duplicating the snapshot shape as a storage-local struct) would create
//! exactly the second, silently-drifting definition this workspace's
//! column convention exists to avoid.
//!
//! A snapshot is stored as its own JSON encoding rather than exploded into
//! columns. Capability shape is provider-driven and will grow (master plan
//! S2.7: discover, do not hard-code); a JSON blob plus the indexed columns
//! actually queried (provider, runtime, capture time) means a new capability
//! field is a code change, not a migration.

use nacc_domain::CapabilitySnapshotId;
use nacc_provider_core::{CapabilitySnapshot, ProviderInstallation, RuntimeLocation};
use rusqlite::{OptionalExtension, Row};

use crate::{lock, Database, Result, StorageError};

fn row_to_installation(row: &Row<'_>) -> std::result::Result<ProviderInstallation, StorageError> {
    let provider_json: String = row.get("provider_json")?;
    let runtime_json: String = row.get("runtime_json")?;
    let installed: i64 = row.get("installed")?;
    Ok(ProviderInstallation {
        provider: serde_json::from_str(&provider_json)?,
        runtime: serde_json::from_str(&runtime_json)?,
        probe: nacc_provider_core::InstallationProbe {
            installed: installed != 0,
            executable_path: row.get("executable_path")?,
            version: row.get("version")?,
        },
        detected_at_millis: row.get::<_, i64>("detected_at_millis")?.max(0) as u64,
    })
}

impl Database {
    /// Record the latest detection result for one provider on one runtime,
    /// replacing any previous detection for the same pair. Detection is a
    /// repeated observation of the *same* fact, so one row per
    /// (provider, runtime) is the truth; a history table here would just
    /// accumulate stale answers to "is it installed?".
    pub async fn upsert_provider_installation(
        &self,
        installation: &ProviderInstallation,
    ) -> Result<()> {
        let installation = installation.clone();
        let conn = self.connection();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            conn.execute(
                "INSERT INTO provider_installations (
                    provider_json, runtime_json, installed, executable_path, version,
                    detected_at_millis
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(provider_json, runtime_json) DO UPDATE SET
                    installed = excluded.installed,
                    executable_path = excluded.executable_path,
                    version = excluded.version,
                    detected_at_millis = excluded.detected_at_millis",
                rusqlite::params![
                    serde_json::to_string(&installation.provider)?,
                    serde_json::to_string(&installation.runtime)?,
                    i64::from(installation.probe.installed),
                    installation.probe.executable_path,
                    installation.probe.version,
                    installation.detected_at_millis as i64,
                ],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn list_provider_installations(&self) -> Result<Vec<ProviderInstallation>> {
        let conn = self.connection();
        tokio::task::spawn_blocking(move || -> Result<Vec<ProviderInstallation>> {
            let conn = lock(&conn);
            let mut stmt = conn.prepare(
                "SELECT provider_json, runtime_json, installed, executable_path, version,
                        detected_at_millis
                 FROM provider_installations ORDER BY provider_json, runtime_json",
            )?;
            let mut rows = stmt.query([])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push(row_to_installation(row)?);
            }
            Ok(out)
        })
        .await?
    }

    /// Persist a capability snapshot and return the id it was stored
    /// under. Snapshots are append-only (unlike installations): the whole
    /// point of `captured_at_millis` is showing how a provider's reported
    /// capabilities *changed*, which requires keeping the old observations.
    pub async fn record_capability_snapshot(
        &self,
        snapshot: &CapabilitySnapshot,
    ) -> Result<CapabilitySnapshotId> {
        let id = CapabilitySnapshotId::new();
        let snapshot = snapshot.clone();
        let conn = self.connection();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            conn.execute(
                "INSERT INTO capability_snapshots (
                    id, provider_json, runtime_json, snapshot_json, captured_at_millis
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    id.to_string(),
                    serde_json::to_string(&snapshot.provider)?,
                    serde_json::to_string(&snapshot.runtime)?,
                    serde_json::to_string(&snapshot)?,
                    snapshot.captured_at_millis as i64,
                ],
            )?;
            Ok(())
        })
        .await??;
        Ok(id)
    }

    /// The most recent snapshot for a provider on a runtime, if any.
    pub async fn latest_capability_snapshot(
        &self,
        provider: nacc_domain::ProviderId,
        runtime: RuntimeLocation,
    ) -> Result<Option<CapabilitySnapshot>> {
        let conn = self.connection();
        let provider_json = serde_json::to_string(&provider)?;
        let runtime_json = serde_json::to_string(&runtime)?;
        tokio::task::spawn_blocking(move || -> Result<Option<CapabilitySnapshot>> {
            let conn = lock(&conn);
            let raw = conn
                .query_row(
                    "SELECT snapshot_json FROM capability_snapshots
                     WHERE provider_json = ?1 AND runtime_json = ?2
                     ORDER BY captured_at_millis DESC, rowid DESC LIMIT 1",
                    rusqlite::params![provider_json, runtime_json],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            match raw {
                Some(json) => Ok(Some(serde_json::from_str(&json)?)),
                None => Ok(None),
            }
        })
        .await?
    }

    /// When each snapshot for a provider was captured, newest first -- the
    /// "has this provider's reported capability set changed?" history the
    /// Providers page shows without materializing every snapshot body.
    pub async fn capability_snapshot_history(
        &self,
        provider: nacc_domain::ProviderId,
    ) -> Result<Vec<(CapabilitySnapshotId, u64)>> {
        let conn = self.connection();
        let provider_json = serde_json::to_string(&provider)?;
        tokio::task::spawn_blocking(move || -> Result<Vec<(CapabilitySnapshotId, u64)>> {
            let conn = lock(&conn);
            let mut stmt = conn.prepare(
                "SELECT id, captured_at_millis FROM capability_snapshots
                 WHERE provider_json = ?1 ORDER BY captured_at_millis DESC, rowid DESC",
            )?;
            let mut rows = stmt.query([provider_json])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                let id: String = row.get(0)?;
                let captured: i64 = row.get(1)?;
                out.push((
                    id.parse().map_err(|e| StorageError::CorruptStoredValue {
                        entity: "capability_snapshots.id",
                        value: id.clone(),
                        detail: format!("{e}"),
                    })?,
                    captured.max(0) as u64,
                ));
            }
            Ok(out)
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nacc_domain::ProviderId;
    use nacc_provider_core::{
        AcpTransport, AuthProbe, CapabilitySnapshot, InstallationProbe, ModelDescriptor,
        ProviderHealth,
    };

    fn installation(version: &str) -> ProviderInstallation {
        ProviderInstallation {
            provider: ProviderId::Claude,
            runtime: RuntimeLocation::NativeWindows,
            probe: InstallationProbe {
                installed: true,
                executable_path: Some("C:\\bin\\claude.exe".into()),
                version: Some(version.into()),
            },
            detected_at_millis: 1_735_000_000_000,
        }
    }

    fn snapshot(version: &str) -> CapabilitySnapshot {
        CapabilitySnapshot {
            provider: ProviderId::Claude,
            runtime: RuntimeLocation::NativeWindows,
            installation: InstallationProbe {
                installed: true,
                executable_path: Some("C:\\bin\\claude.exe".into()),
                version: Some(version.into()),
            },
            auth: AuthProbe {
                authenticated: true,
                account_label: Some("user".into()),
                detail: None,
            },
            health: ProviderHealth::Ready,
            models: vec![ModelDescriptor {
                id: "claude-fable-5".into(),
                display_name: "Fable 5".into(),
                reasoning_levels: vec![],
                thinking: nacc_domain::ThinkingMode::Unsupported,
                structured_output: true,
                context_window_tokens: Some(200_000),
            }],
            noninteractive_mode: true,
            structured_json_output: true,
            streaming_json_output: true,
            interactive_pty: false,
            session_resume: true,
            custom_agents: true,
            subagents: true,
            mcp: true,
            acp_transport: AcpTransport::Unverified,
            usage_reporting: true,
            cancellation_documented: true,
            captured_at_millis: 1_735_000_000_000,
        }
    }

    #[tokio::test]
    async fn recording_an_installation_twice_updates_rather_than_duplicates() {
        let db = Database::open_in_memory().unwrap();
        db.upsert_provider_installation(&installation("1.0.0"))
            .await
            .unwrap();
        db.upsert_provider_installation(&installation("1.0.1"))
            .await
            .unwrap();

        let all = db.list_provider_installations().await.unwrap();
        assert_eq!(all.len(), 1, "one row per (provider, runtime)");
        assert_eq!(all[0].probe.version.as_deref(), Some("1.0.1"));
    }

    #[tokio::test]
    async fn a_missing_executable_path_round_trips_as_absent() {
        let db = Database::open_in_memory().unwrap();
        let mut record = installation("0.0.0");
        record.probe.installed = false;
        record.probe.executable_path = None;
        record.probe.version = None;
        db.upsert_provider_installation(&record).await.unwrap();

        let back = &db.list_provider_installations().await.unwrap()[0];
        assert!(!back.probe.installed);
        assert!(back.probe.executable_path.is_none());
    }

    #[tokio::test]
    async fn capability_snapshots_are_append_only_and_latest_wins() {
        let db = Database::open_in_memory().unwrap();
        let mut older = snapshot("1.0.0");
        older.captured_at_millis = 1_000;
        let mut newer = snapshot("1.1.0");
        newer.captured_at_millis = 2_000;

        db.record_capability_snapshot(&older).await.unwrap();
        db.record_capability_snapshot(&newer).await.unwrap();

        let latest = db
            .latest_capability_snapshot(ProviderId::Claude, RuntimeLocation::NativeWindows)
            .await
            .unwrap()
            .expect("a recorded snapshot must be retrievable");
        assert_eq!(
            latest.installation.version.as_deref(),
            Some("1.1.0"),
            "the newest capture must win"
        );
        assert_eq!(latest.models.len(), 1);
        assert_eq!(latest.health, ProviderHealth::Ready);

        let history = db
            .capability_snapshot_history(ProviderId::Claude)
            .await
            .unwrap();
        assert_eq!(history.len(), 2, "history keeps both captures");
        assert_eq!(history[0].1, 2_000, "history is newest first");
    }

    #[tokio::test]
    async fn an_unknown_provider_has_no_snapshot_and_no_history() {
        let db = Database::open_in_memory().unwrap();
        assert!(db
            .latest_capability_snapshot(ProviderId::Opencode, RuntimeLocation::Wsl2)
            .await
            .unwrap()
            .is_none());
        assert!(db
            .capability_snapshot_history(ProviderId::Opencode)
            .await
            .unwrap()
            .is_empty());
    }
}

//! Durable persistence for `nacc_events::AuditRecord` (master plan S4.4's
//! "audit records" data group; S7's `AuditEvent`; S22's required
//! audit-record fields). Pure I/O, same split as `events.rs`: the record
//! shape lives in `nacc-events`, this module only stores and retrieves it.

use rusqlite::{params, Row};

use nacc_domain::{
    AttemptId, AuditEventId, ModelId, NodeRunId, PermissionProfile, ProjectId, ProviderId,
    ReasoningLevel, WorkflowRunId,
};
use nacc_events::AuditRecord;

use crate::{lock, Database, Result, StorageError};

const SELECT_COLUMNS: &str = "id, actor, action, project_id, workflow_run_id, node_run_id, \
     attempt_id, requested_provider_json, actual_provider_json, requested_model, \
     actual_model, effective_reasoning_level_json, effective_permission_profile_json, \
     command_executable, redacted_arguments_json, working_directory, created_at_millis";

struct RawAuditRow {
    id: String,
    actor: String,
    action: String,
    project_id: Option<String>,
    workflow_run_id: Option<String>,
    node_run_id: Option<String>,
    attempt_id: Option<String>,
    requested_provider_json: Option<String>,
    actual_provider_json: Option<String>,
    requested_model: Option<String>,
    actual_model: Option<String>,
    effective_reasoning_level_json: Option<String>,
    effective_permission_profile_json: Option<String>,
    command_executable: Option<String>,
    redacted_arguments_json: String,
    working_directory: Option<String>,
    created_at_millis: i64,
}

fn row_to_raw(row: &Row<'_>) -> rusqlite::Result<RawAuditRow> {
    Ok(RawAuditRow {
        id: row.get(0)?,
        actor: row.get(1)?,
        action: row.get(2)?,
        project_id: row.get(3)?,
        workflow_run_id: row.get(4)?,
        node_run_id: row.get(5)?,
        attempt_id: row.get(6)?,
        requested_provider_json: row.get(7)?,
        actual_provider_json: row.get(8)?,
        requested_model: row.get(9)?,
        actual_model: row.get(10)?,
        effective_reasoning_level_json: row.get(11)?,
        effective_permission_profile_json: row.get(12)?,
        command_executable: row.get(13)?,
        redacted_arguments_json: row.get(14)?,
        working_directory: row.get(15)?,
        created_at_millis: row.get(16)?,
    })
}

fn parse_id<T: std::str::FromStr>(entity: &'static str, value: &str) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    value.parse().map_err(|e| StorageError::CorruptStoredValue {
        entity,
        value: value.to_string(),
        detail: format!("{e}"),
    })
}

fn raw_to_audit_record(raw: RawAuditRow) -> Result<AuditRecord> {
    Ok(AuditRecord {
        id: parse_id::<AuditEventId>("AuditRecord.id", &raw.id)?,
        actor: raw.actor,
        action: raw.action,
        project_id: raw
            .project_id
            .as_deref()
            .map(|s| parse_id::<ProjectId>("AuditRecord.project_id", s))
            .transpose()?,
        workflow_run_id: raw
            .workflow_run_id
            .as_deref()
            .map(|s| parse_id::<WorkflowRunId>("AuditRecord.workflow_run_id", s))
            .transpose()?,
        node_run_id: raw
            .node_run_id
            .as_deref()
            .map(|s| parse_id::<NodeRunId>("AuditRecord.node_run_id", s))
            .transpose()?,
        attempt_id: raw
            .attempt_id
            .as_deref()
            .map(|s| parse_id::<AttemptId>("AuditRecord.attempt_id", s))
            .transpose()?,
        requested_provider: raw
            .requested_provider_json
            .as_deref()
            .map(serde_json::from_str::<ProviderId>)
            .transpose()?,
        actual_provider: raw
            .actual_provider_json
            .as_deref()
            .map(serde_json::from_str::<ProviderId>)
            .transpose()?,
        requested_model: raw.requested_model.map(ModelId::from),
        actual_model: raw.actual_model.map(ModelId::from),
        effective_reasoning_level: raw
            .effective_reasoning_level_json
            .as_deref()
            .map(serde_json::from_str::<ReasoningLevel>)
            .transpose()?,
        effective_permission_profile: raw
            .effective_permission_profile_json
            .as_deref()
            .map(serde_json::from_str::<PermissionProfile>)
            .transpose()?,
        command_executable: raw.command_executable,
        redacted_arguments: serde_json::from_str(&raw.redacted_arguments_json)?,
        working_directory: raw.working_directory,
        created_at_millis: raw.created_at_millis as u64,
    })
}

impl Database {
    pub async fn append_audit_record(&self, record: &AuditRecord) -> Result<()> {
        let conn = self.connection();
        let record = record.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            conn.execute(
                &format!(
                    "INSERT INTO audit_events ({SELECT_COLUMNS}) VALUES \
                     (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)"
                ),
                params![
                    record.id.to_string(),
                    record.actor,
                    record.action,
                    record.project_id.map(|id| id.to_string()),
                    record.workflow_run_id.map(|id| id.to_string()),
                    record.node_run_id.map(|id| id.to_string()),
                    record.attempt_id.map(|id| id.to_string()),
                    record
                        .requested_provider
                        .map(|p| serde_json::to_string(&p))
                        .transpose()?,
                    record
                        .actual_provider
                        .map(|p| serde_json::to_string(&p))
                        .transpose()?,
                    record.requested_model.as_ref().map(|m| m.0.clone()),
                    record.actual_model.as_ref().map(|m| m.0.clone()),
                    record
                        .effective_reasoning_level
                        .map(|r| serde_json::to_string(&r))
                        .transpose()?,
                    record
                        .effective_permission_profile
                        .map(|p| serde_json::to_string(&p))
                        .transpose()?,
                    record.command_executable,
                    serde_json::to_string(&record.redacted_arguments)?,
                    record.working_directory,
                    record.created_at_millis as i64,
                ],
            )?;
            Ok(())
        })
        .await
        .expect("storage worker thread panicked")
    }

    /// Audit records for one workflow run, oldest first (master plan S22's
    /// "workflow run" correlation ID).
    pub async fn list_audit_records_for_workflow_run(
        &self,
        workflow_run_id: WorkflowRunId,
    ) -> Result<Vec<AuditRecord>> {
        let conn = self.connection();
        let id_str = workflow_run_id.to_string();
        let raws = tokio::task::spawn_blocking(move || -> rusqlite::Result<Vec<RawAuditRow>> {
            let conn = lock(&conn);
            let mut stmt = conn.prepare(&format!(
                "SELECT {SELECT_COLUMNS} FROM audit_events WHERE workflow_run_id = ?1 ORDER BY created_at_millis"
            ))?;
            stmt.query_map([id_str], row_to_raw)?.collect()
        })
        .await
        .expect("storage worker thread panicked")?;

        raws.into_iter().map(raw_to_audit_record).collect()
    }
}

#[cfg(test)]
mod tests {
    use nacc_domain::{ProviderId, ReasoningLevel, WorkflowRunId};
    use nacc_events::AuditRecord;

    use crate::Database;

    #[tokio::test]
    async fn appended_record_round_trips_with_all_optional_fields_set() {
        let db = Database::open_in_memory().unwrap();
        let run_id = WorkflowRunId::new();
        let mut record = AuditRecord::new(
            "role_matrix".to_string(),
            "launch_attempt".to_string(),
            None,
            Some(run_id),
            None,
            None,
            1_735_000_000_000,
        );
        record.requested_provider = Some(ProviderId::Claude);
        record.actual_provider = Some(ProviderId::Claude);
        record.effective_reasoning_level = Some(ReasoningLevel::High);
        record.command_executable = Some("claude.exe".to_string());
        record.redacted_arguments = vec!["--effort".to_string(), "high".to_string()];
        record.working_directory = Some("D:\\Projects\\NACC".to_string());

        db.append_audit_record(&record).await.unwrap();

        let found = db
            .list_audit_records_for_workflow_run(run_id)
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, record.id);
        assert_eq!(found[0].requested_provider, Some(ProviderId::Claude));
        assert_eq!(found[0].redacted_arguments, record.redacted_arguments);
        assert_eq!(
            found[0].working_directory.as_deref(),
            Some("D:\\Projects\\NACC")
        );
    }

    #[tokio::test]
    async fn minimal_record_with_no_optional_fields_round_trips_as_absent() {
        let db = Database::open_in_memory().unwrap();
        let run_id = WorkflowRunId::new();
        let record = AuditRecord::new(
            "system".to_string(),
            "startup".to_string(),
            None,
            Some(run_id),
            None,
            None,
            1_735_000_000_000,
        );

        db.append_audit_record(&record).await.unwrap();

        let found = &db
            .list_audit_records_for_workflow_run(run_id)
            .await
            .unwrap()[0];
        assert!(found.requested_provider.is_none());
        assert!(found.redacted_arguments.is_empty());
        assert!(found.working_directory.is_none());
    }
}

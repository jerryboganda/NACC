//! Read-only audit-log IPC. Audit records are security evidence, so this
//! surface deliberately exposes bounded retrieval only; creation remains
//! inside the privileged backend paths that perform the audited actions.

use serde::{Deserialize, Serialize};
use tauri::State;

use nacc_domain::{
    AttemptId, AuditEventId, ModelId, NodeRunId, PermissionProfile, ProjectId, ProviderId,
    ReasoningLevel, WorkflowRunId,
};
use nacc_events::AuditRecord as StoredAuditRecord;

use crate::AppState;

const DEFAULT_AUDIT_LIMIT: u32 = 100;
const MAX_AUDIT_LIMIT: u32 = 500;

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct ListAuditRecordsArgs {
    pub workflow_run_id: Option<WorkflowRunId>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct AuditRecordView {
    pub id: AuditEventId,
    pub actor: String,
    pub action: String,
    pub project_id: Option<ProjectId>,
    pub workflow_run_id: Option<WorkflowRunId>,
    pub node_run_id: Option<NodeRunId>,
    pub attempt_id: Option<AttemptId>,
    pub requested_provider: Option<ProviderId>,
    pub actual_provider: Option<ProviderId>,
    pub requested_model: Option<ModelId>,
    pub actual_model: Option<ModelId>,
    pub effective_reasoning_level: Option<ReasoningLevel>,
    pub effective_permission_profile: Option<PermissionProfile>,
    pub command_executable: Option<String>,
    pub redacted_arguments: Vec<String>,
    pub working_directory: Option<String>,
    pub created_at_millis: String,
}

impl From<StoredAuditRecord> for AuditRecordView {
    fn from(record: StoredAuditRecord) -> Self {
        Self {
            id: record.id,
            actor: record.actor,
            action: record.action,
            project_id: record.project_id,
            workflow_run_id: record.workflow_run_id,
            node_run_id: record.node_run_id,
            attempt_id: record.attempt_id,
            requested_provider: record.requested_provider,
            actual_provider: record.actual_provider,
            requested_model: record.requested_model,
            actual_model: record.actual_model,
            effective_reasoning_level: record.effective_reasoning_level,
            effective_permission_profile: record.effective_permission_profile,
            command_executable: record.command_executable,
            redacted_arguments: record.redacted_arguments,
            working_directory: record.working_directory,
            created_at_millis: record.created_at_millis.to_string(),
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_audit_records(
    args: ListAuditRecordsArgs,
    state: State<'_, AppState>,
) -> Result<Vec<AuditRecordView>, String> {
    let limit = args
        .limit
        .unwrap_or(DEFAULT_AUDIT_LIMIT)
        .clamp(1, MAX_AUDIT_LIMIT);

    let records = match args.workflow_run_id {
        Some(run_id) => state
            .storage
            .list_recent_audit_records_for_workflow_run(run_id, limit)
            .await
            .map_err(|e| e.to_string()),
        None => state
            .storage
            .list_recent_audit_records(limit)
            .await
            .map_err(|e| e.to_string()),
    }?;

    Ok(records.into_iter().map(Into::into).collect())
}

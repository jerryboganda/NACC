//! Read-only worktree lease visibility for the desktop UI. Lifecycle
//! mutations stay behind the workflow engine/worktree manager so the UI
//! cannot bypass release/quarantine safety checks.

use serde::{Deserialize, Serialize};
use tauri::State;

use nacc_domain::{
    NodeRunId, ProjectId, WorkflowRunId, WorktreeLease, WorktreeLeaseId, WorktreeState,
};

use crate::AppState;

const DEFAULT_WORKTREE_LIMIT: u32 = 100;
const MAX_WORKTREE_LIMIT: u32 = 500;

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct ListWorktreeLeasesArgs {
    pub project_id: Option<ProjectId>,
    #[serde(default)]
    pub active_only: bool,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct WorktreeLeaseView {
    pub id: WorktreeLeaseId,
    pub project_id: ProjectId,
    pub workflow_run_id: Option<WorkflowRunId>,
    pub node_run_id: Option<NodeRunId>,
    pub path: String,
    pub branch: String,
    pub base_commit: String,
    pub head_commit: Option<String>,
    pub state: WorktreeState,
    pub owner_process_id: Option<u32>,
    pub quarantine_reason: Option<String>,
    pub created_at_millis: String,
    pub updated_at_millis: String,
}

impl From<WorktreeLease> for WorktreeLeaseView {
    fn from(lease: WorktreeLease) -> Self {
        Self {
            id: lease.id,
            project_id: lease.project_id,
            workflow_run_id: lease.workflow_run_id,
            node_run_id: lease.node_run_id,
            path: lease.path,
            branch: lease.branch,
            base_commit: lease.base_commit,
            head_commit: lease.head_commit,
            state: lease.state,
            owner_process_id: lease.owner_process_id,
            quarantine_reason: lease.quarantine_reason,
            created_at_millis: lease.created_at_millis.to_string(),
            updated_at_millis: lease.updated_at_millis.to_string(),
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_worktree_leases(
    args: ListWorktreeLeasesArgs,
    state: State<'_, AppState>,
) -> Result<Vec<WorktreeLeaseView>, String> {
    let limit = args
        .limit
        .unwrap_or(DEFAULT_WORKTREE_LIMIT)
        .clamp(1, MAX_WORKTREE_LIMIT);

    state
        .storage
        .list_recent_worktree_leases(args.project_id, args.active_only, limit)
        .await
        .map(|leases| leases.into_iter().map(Into::into).collect())
        .map_err(|e| e.to_string())
}

//! Workflow engine IPC (master plan S17.7's Run Console): start, inspect,
//! pause, cancel, resume, and approve real durable runs, over the Phase 7
//! engine that until now had no application wiring at all.
//!
//! # Why starting a run is deliberately two steps
//!
//! `start_workflow_run` persists the run and its nodes, then drives it in a
//! background task. The command returns as soon as the run exists, so the
//! GUI shows a run immediately instead of freezing for the length of an
//! agent session -- and if NACC is closed mid-run, what is in SQLite is a
//! real interrupted run that recovery can explain, not an in-memory ghost.

use serde::{Deserialize, Serialize};
use tauri::State;

use nacc_domain::{ApprovalDecision, ApprovalId, ProjectId, RunState, WorkflowRunId};
use nacc_orchestrator::{built_in_template, built_in_templates, RunSnapshot};
use nacc_storage::{CheckpointRecord, NodeRunRecord, WorkflowRunRecord};

use crate::AppState;

/// Every state, so "all runs" is expressed as a filter rather than a special
/// case in storage.
const ALL_STATES: [RunState; 8] = [
    RunState::Pending,
    RunState::Running,
    RunState::Paused,
    RunState::AwaitingApproval,
    RunState::Interrupted,
    RunState::Succeeded,
    RunState::Failed,
    RunState::Cancelled,
];

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct WorkflowTemplateView {
    pub name: String,
    pub description: String,
    pub node_keys: Vec<String>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct WorkflowRunView {
    pub id: WorkflowRunId,
    pub project_id: ProjectId,
    pub template_name: String,
    pub state: RunState,
    pub note: Option<String>,
    pub created_at_millis: String,
    pub updated_at_millis: String,
}

impl From<WorkflowRunRecord> for WorkflowRunView {
    fn from(run: WorkflowRunRecord) -> Self {
        Self {
            id: run.id,
            project_id: run.project_id,
            template_name: run.template_name,
            state: run.state,
            note: run.note,
            created_at_millis: run.created_at_millis.to_string(),
            updated_at_millis: run.updated_at_millis.to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct NodeRunView {
    pub id: nacc_domain::NodeRunId,
    pub node_key: String,
    pub title: String,
    pub state: nacc_domain::NodeState,
    pub attempts: u32,
    /// Whether this node must be approved by a human before it runs.
    pub requires_approval: bool,
    pub last_detail: Option<String>,
}

impl From<NodeRunRecord> for NodeRunView {
    fn from(node: NodeRunRecord) -> Self {
        Self {
            id: node.id,
            node_key: node.node_key,
            title: node.title,
            state: node.state,
            attempts: node.attempts,
            requires_approval: node.definition.requires_approval,
            last_detail: node.last_detail,
        }
    }
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct ApprovalView {
    pub id: ApprovalId,
    pub node_run_id: nacc_domain::NodeRunId,
    pub node_key: String,
    pub summary: String,
    /// `None` while the gate is still open.
    pub decision: Option<String>,
    pub decided_at_millis: Option<String>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct CheckpointView {
    pub sequence: u32,
    pub state: RunState,
    pub detail: String,
    pub created_at_millis: String,
}

impl From<CheckpointRecord> for CheckpointView {
    fn from(checkpoint: CheckpointRecord) -> Self {
        Self {
            sequence: checkpoint.sequence,
            state: checkpoint.state,
            detail: checkpoint.detail,
            created_at_millis: checkpoint.created_at_millis.to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct RunSnapshotView {
    pub run: WorkflowRunView,
    pub nodes: Vec<NodeRunView>,
    pub approvals: Vec<ApprovalView>,
    pub checkpoints: Vec<CheckpointView>,
    /// `u32`, not `usize`: specta refuses to export pointer-width integers
    /// (precision loss across the IPC boundary), matching `attempts`/`sequence`.
    pub pending_approval_count: u32,
    pub finished: bool,
}

fn snapshot_view(snapshot: &RunSnapshot) -> RunSnapshotView {
    let approvals = snapshot
        .approvals
        .iter()
        .map(|approval| {
            let node_key = snapshot
                .nodes
                .iter()
                .find(|node| node.id == approval.node_run_id)
                .map(|node| node.node_key.clone())
                .unwrap_or_else(|| "<unknown node>".to_string());
            ApprovalView {
                id: approval.id,
                node_run_id: approval.node_run_id,
                node_key,
                summary: approval.summary.clone(),
                decision: approval.decision.as_ref().map(|decision| match decision {
                    ApprovalDecision::Approved { by } => format!("approved by {by}"),
                    ApprovalDecision::Rejected { by, reason } => {
                        format!("rejected by {by}: {reason}")
                    }
                }),
                decided_at_millis: approval.decided_at_millis.map(|m| m.to_string()),
            }
        })
        .collect();

    RunSnapshotView {
        run: WorkflowRunView::from(snapshot.run.clone()),
        nodes: snapshot
            .nodes
            .iter()
            .cloned()
            .map(NodeRunView::from)
            .collect(),
        approvals,
        checkpoints: snapshot
            .checkpoints
            .iter()
            .cloned()
            .map(CheckpointView::from)
            .collect(),
        pending_approval_count: snapshot.pending_approvals().len() as u32,
        finished: snapshot.is_finished(),
    }
}

/// One normalized event, as the run console lists it. The payload is carried
/// as JSON text: its shape is provider-driven and the GUI shows it verbatim
/// rather than pretending to know every variant's fields (master plan S8.2).
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct WorkflowEventView {
    pub event_type: nacc_events::EventType,
    pub payload_json: String,
    pub created_at_millis: String,
}

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct StartRunArgs {
    pub project_id: ProjectId,
    pub template_name: String,
    /// Absolute path the agents work in, chosen by the user. Required: an
    /// implicit working directory is how an agent ends up writing somewhere
    /// nobody asked for.
    pub workspace: String,
}

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct RunIdArgs {
    pub run_id: WorkflowRunId,
}

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct ReasonArgs {
    pub run_id: WorkflowRunId,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct DecideApprovalArgs {
    pub run_id: WorkflowRunId,
    pub approval_id: ApprovalId,
    pub approved: bool,
    /// Who decided. Required even for approval: "who let this run write to
    /// the repository" must have an answer (master plan S22).
    pub by: String,
    /// Required when rejecting, ignored otherwise.
    pub reason: Option<String>,
    /// Whether to continue the run immediately after approving.
    pub resume_after: bool,
}

#[tauri::command]
#[specta::specta]
pub fn list_workflow_templates() -> Vec<WorkflowTemplateView> {
    built_in_templates()
        .into_iter()
        .map(|template| WorkflowTemplateView {
            name: template.name,
            description: template.description,
            node_keys: template.nodes.into_iter().map(|node| node.key).collect(),
        })
        .collect()
}

/// Drive a run in the background. Returns nothing: the caller already has the
/// run's durable state, and the run's progress is observed by reading it back
/// (so the GUI never depends on a single long IPC call staying alive).
fn drive_in_background(state: &AppState, run_id: WorkflowRunId) {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(err) = engine.run(run_id).await {
            // Reported, not swallowed: a run that cannot advance is exactly
            // what the operator needs to see.
            tracing::error!(run = %run_id, error = %err, "driving a workflow run failed");
        }
    });
}

#[tauri::command]
#[specta::specta]
pub async fn start_workflow_run(
    args: StartRunArgs,
    state: State<'_, AppState>,
) -> Result<RunSnapshotView, String> {
    let template = built_in_template(&args.template_name).ok_or_else(|| {
        let available: Vec<String> = built_in_templates()
            .into_iter()
            .map(|template| template.name)
            .collect();
        format!(
            "unknown workflow template `{}`; available: {}",
            args.template_name,
            available.join(", ")
        )
    })?;

    let workspace = crate::routing::RoleMatrixRouting::validate_workspace(std::path::Path::new(
        &args.workspace,
    ))?;
    state.routing.set_workspace(args.project_id, workspace);

    let snapshot = state
        .engine
        .start_run(args.project_id, &template)
        .await
        .map_err(|e| e.to_string())?;

    drive_in_background(&state, snapshot.run.id);
    Ok(snapshot_view(&snapshot))
}

#[tauri::command]
#[specta::specta]
pub async fn get_workflow_run(
    args: RunIdArgs,
    state: State<'_, AppState>,
) -> Result<RunSnapshotView, String> {
    state
        .engine
        .snapshot(args.run_id)
        .await
        .map(|snapshot| snapshot_view(&snapshot))
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_workflow_runs(
    state: State<'_, AppState>,
) -> Result<Vec<WorkflowRunView>, String> {
    state
        .storage
        .list_workflow_runs_in_states(&ALL_STATES)
        .await
        .map(|runs| runs.into_iter().map(WorkflowRunView::from).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn pause_workflow_run(
    args: ReasonArgs,
    state: State<'_, AppState>,
) -> Result<RunSnapshotView, String> {
    state
        .engine
        .pause(args.run_id, &args.reason)
        .await
        .map(|snapshot| snapshot_view(&snapshot))
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_workflow_run(
    args: ReasonArgs,
    state: State<'_, AppState>,
) -> Result<RunSnapshotView, String> {
    state
        .engine
        .cancel(args.run_id, &args.reason)
        .await
        .map(|snapshot| snapshot_view(&snapshot))
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn resume_workflow_run(
    args: RunIdArgs,
    state: State<'_, AppState>,
) -> Result<RunSnapshotView, String> {
    // Requeueing unfinished work must be visible before the run continues, so
    // only the driving is deferred to a background task.
    let snapshot = state
        .engine
        .snapshot(args.run_id)
        .await
        .map_err(|e| e.to_string())?;
    if snapshot.run.state == RunState::AwaitingApproval {
        return Err(
            "this run is waiting on an approval decision; decide the approval instead of resuming"
                .to_string(),
        );
    }
    let engine = state.engine.clone();
    let run_id = args.run_id;
    tauri::async_runtime::spawn(async move {
        if let Err(err) = engine.resume(run_id).await {
            tracing::error!(run = %run_id, error = %err, "resuming a workflow run failed");
        }
    });
    Ok(snapshot_view(&snapshot))
}

#[tauri::command]
#[specta::specta]
pub async fn decide_workflow_approval(
    args: DecideApprovalArgs,
    state: State<'_, AppState>,
) -> Result<RunSnapshotView, String> {
    let decision = if args.approved {
        ApprovalDecision::Approved {
            by: args.by.clone(),
        }
    } else {
        ApprovalDecision::Rejected {
            by: args.by.clone(),
            reason: args
                .reason
                .clone()
                .ok_or_else(|| "a rejection must state a reason".to_string())?,
        }
    };

    state
        .engine
        .decide_approval(args.run_id, args.approval_id, &decision)
        .await
        .map_err(|e| e.to_string())?;

    if args.approved && args.resume_after {
        drive_in_background(&state, args.run_id);
    }

    state
        .engine
        .snapshot(args.run_id)
        .await
        .map(|snapshot| snapshot_view(&snapshot))
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_workflow_events(
    args: RunIdArgs,
    state: State<'_, AppState>,
) -> Result<Vec<WorkflowEventView>, String> {
    state
        .storage
        .list_events_for_workflow_run(args.run_id)
        .await
        .map(|events| {
            events
                .into_iter()
                .map(|event| WorkflowEventView {
                    event_type: event.event_type,
                    payload_json: event.payload.to_string(),
                    created_at_millis: event.created_at_millis.to_string(),
                })
                .collect()
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_template_is_listed_with_its_nodes() {
        let views = list_workflow_templates();
        assert!(!views.is_empty(), "the built-in presets must be visible");
        for view in &views {
            assert!(
                !view.node_keys.is_empty(),
                "{} has no nodes, so it cannot be run",
                view.name
            );
        }
    }

    #[test]
    fn an_unknown_template_name_does_not_resolve() {
        assert!(built_in_template("no-such-template").is_none());
    }

    #[test]
    fn all_states_covers_the_whole_durable_state_set() {
        // "All runs" must not silently omit a state a run can be in.
        for state in [
            RunState::Pending,
            RunState::Running,
            RunState::Paused,
            RunState::AwaitingApproval,
            RunState::Interrupted,
            RunState::Succeeded,
            RunState::Failed,
            RunState::Cancelled,
        ] {
            assert!(ALL_STATES.contains(&state), "{state:?} is missing");
        }
    }
}

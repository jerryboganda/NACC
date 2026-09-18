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
use std::collections::HashMap;
use tauri::State;

use nacc_domain::{
    ApprovalDecision, ApprovalId, ProjectId, RunState, WorkflowRunId, WorkflowTemplate,
};
use nacc_orchestrator::{built_in_template, built_in_templates, RunSnapshot};
use nacc_storage::{CheckpointRecord, NodeRunRecord, WorkflowRunRecord, WorkflowTemplateRecord};

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
    /// Storage edition of this graph; advances when the definition changes.
    pub version: u32,
    /// Built-in presets are protected: the GUI can run them but never
    /// replace or delete them.
    pub is_built_in: bool,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct WorkflowTemplateDefinitionView {
    pub name: String,
    pub description: String,
    pub nodes: Vec<nacc_domain::WorkflowNode>,
    pub version: u32,
    pub is_built_in: bool,
}

fn template_view(
    name: &str,
    description: &str,
    nodes: &[nacc_domain::WorkflowNode],
    version: u32,
    is_built_in: bool,
) -> WorkflowTemplateView {
    WorkflowTemplateView {
        name: name.to_string(),
        description: description.to_string(),
        node_keys: nodes.iter().map(|node| node.key.clone()).collect(),
        version,
        is_built_in,
    }
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
    /// Absolute path to a directory under which a fresh git worktree lease
    /// is allocated for this run (master plan S16). When set, every node of
    /// the run works in that isolated worktree and the primary checkout is
    /// never touched. `None` runs directly in `workspace`.
    pub worktrees_root: Option<String>,
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

/// Drive a run in the background. Returns nothing: the caller already has the
/// run's durable state, and the run's progress is observed by reading it back
/// (so the GUI never depends on a single long IPC call staying alive). When
/// the run reaches a terminal state in this session, its worktree lease (if
/// any) is released -- best-effort, never changing the run's own state.
fn drive_in_background(state: &AppState, run_id: WorkflowRunId) {
    let engine = state.engine.clone();
    let worktrees = state.worktrees.clone();
    let leases = state.run_leases.clone();
    let routing = state.routing.clone();
    tauri::async_runtime::spawn(async move {
        let result = engine.run(run_id).await;
        if let Ok(snapshot) = &result {
            if snapshot.is_finished() {
                release_run_lease(&worktrees, &leases, &routing, run_id).await;
            }
        }
        if let Err(err) = result {
            // Reported, not swallowed: a run that cannot advance is exactly
            // what the operator needs to see.
            tracing::error!(run = %run_id, error = %err, "driving a workflow run failed");
        }
    });
}

/// Release a run's worktree lease and drop its routing override. Clean trees
/// are removed; anything preserved is quarantined by the worktree crate's
/// own release policy rather than destroyed (master plan S16). A failure
/// here leaves the durable lease row for startup reconciliation -- it never
/// blocks or rewrites the run.
pub(crate) async fn release_run_lease(
    worktrees: &nacc_worktree::WorktreeManager,
    leases: &std::sync::Mutex<HashMap<WorkflowRunId, nacc_domain::WorktreeLease>>,
    routing: &crate::routing::RoleMatrixRouting,
    run_id: WorkflowRunId,
) {
    let lease = match leases.lock() {
        Ok(mut leases) => leases.remove(&run_id),
        Err(_) => {
            tracing::error!(
                run = %run_id,
                "run lease registry is poisoned; leaving the durable lease for startup reconciliation"
            );
            routing.clear_run_workspace(run_id);
            return;
        }
    };
    routing.clear_run_workspace(run_id);
    let Some(lease) = lease else { return };
    let repo = match nacc_git::GitRepository::open(&lease.path).await {
        Ok(repo) => repo,
        Err(err) => {
            tracing::warn!(
                lease = %lease.id,
                error = %err,
                "could not reopen a finished run's worktree for release; leaving it to reconciliation"
            );
            return;
        }
    };
    match worktrees
        .release(&repo, &lease, nacc_worktree::ReleasePolicy::RemoveIfSafe)
        .await
    {
        Ok(report) => {
            tracing::info!(
                lease = %lease.id,
                outcome = ?report.outcome,
                "released a finished run's worktree lease"
            );
        }
        Err(err) => {
            tracing::warn!(
                lease = %lease.id,
                error = %err,
                "releasing a finished run's worktree lease failed; the durable lease row stays for reconciliation"
            );
        }
    }
}

/// Allocate a git worktree lease for a whole run (master plan S16): one
/// worktree off the chosen repository, every node of the run working inside
/// it. The base is the repository's current branch; the worktree crate
/// records the exact commit, which is what drift detection compares against.
async fn allocate_run_worktree(
    state: &AppState,
    snapshot: &RunSnapshot,
    template_name: &str,
    workspace: &std::path::Path,
    root: &std::path::Path,
) -> Result<nacc_domain::WorktreeLease, String> {
    let repo = nacc_git::GitRepository::open(workspace)
        .await
        .map_err(|e| format!("the workspace is not a git repository: {e}"))?;
    let base = repo
        .current_branch()
        .await
        .unwrap_or_else(|_| "HEAD".to_string());
    state
        .worktrees
        .allocate(
            &repo,
            nacc_worktree::AllocateRequest {
                project_id: snapshot.run.project_id,
                workflow_run_id: Some(snapshot.run.id),
                node_run_id: None,
                label: template_name.to_string(),
                base,
                worktrees_root: root.to_path_buf(),
            },
        )
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_workflow_templates(
    state: State<'_, AppState>,
) -> Result<Vec<WorkflowTemplateView>, String> {
    let records = state
        .storage
        .list_workflow_templates()
        .await
        .map_err(|e| e.to_string())?;
    if !records.is_empty() {
        return Ok(records
            .iter()
            .map(|record| {
                template_view(
                    &record.name,
                    &record.description,
                    &record.definition.nodes,
                    record.version,
                    record.is_built_in,
                )
            })
            .collect());
    }
    // The startup sync has not landed yet (a fresh database whose sync task
    // is still queued). Fall back to the code-defined presets rather than
    // showing an empty catalog, which would lie about what this build runs.
    Ok(built_in_templates()
        .iter()
        .map(|template| {
            template_view(
                &template.name,
                &template.description,
                &template.nodes,
                1,
                true,
            )
        })
        .collect())
}

/// Load the complete graph for one template so the GUI can inspect or edit
/// it without reconstructing node settings from the summary list.
#[tauri::command]
#[specta::specta]
pub async fn get_workflow_template(
    args: TemplateNameArgs,
    state: State<'_, AppState>,
) -> Result<WorkflowTemplateDefinitionView, String> {
    let name = args.name.trim();
    if name.is_empty() {
        return Err("template name must not be empty".to_string());
    }

    if let Some(record) = state
        .storage
        .get_workflow_template(name)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(WorkflowTemplateDefinitionView {
            name: record.name,
            description: record.description,
            nodes: record.definition.nodes,
            version: record.version,
            is_built_in: record.is_built_in,
        });
    }

    let template =
        built_in_template(name).ok_or_else(|| format!("unknown workflow template `{name}`"))?;
    Ok(WorkflowTemplateDefinitionView {
        name: template.name,
        description: template.description,
        nodes: template.nodes,
        version: 1,
        is_built_in: true,
    })
}

/// Create or update a *custom* workflow template. The DAG is validated
/// before it is stored, so a graph that could never run cannot even be
/// saved; built-in names are refused because presets are the product's own
/// contract with the user.
#[tauri::command]
#[specta::specta]
pub async fn save_workflow_template(
    args: SaveTemplateArgs,
    state: State<'_, AppState>,
) -> Result<WorkflowTemplateView, String> {
    let name = args.name.trim();
    if name.is_empty() {
        return Err("template name must not be empty".to_string());
    }
    if built_in_template(name).is_some() {
        return Err(format!(
            "`{name}` is a built-in preset; save custom templates under a different name"
        ));
    }
    if args.nodes.is_empty() {
        return Err("a template needs at least one node".to_string());
    }
    for node in &args.nodes {
        if node.key.trim().is_empty() {
            return Err("every node needs a key".to_string());
        }
        if node.instruction.trim().is_empty() {
            return Err(format!("node `{}` needs an instruction", node.key));
        }
    }
    nacc_orchestrator::scheduler::validate(&args.nodes).map_err(|e| e.to_string())?;

    let description = args.description.trim().to_string();
    let version = state
        .storage
        .next_workflow_template_version(name)
        .await
        .map_err(|e| e.to_string())?;
    let now = crate::now_millis();
    let record = WorkflowTemplateRecord {
        name: name.to_string(),
        description: description.clone(),
        version,
        is_built_in: false,
        definition: WorkflowTemplate {
            name: name.to_string(),
            description: description.clone(),
            nodes: args.nodes,
        },
        created_at_millis: now,
        updated_at_millis: now,
    };
    state
        .storage
        .upsert_workflow_template(&record)
        .await
        .map_err(|e| e.to_string())?;
    Ok(template_view(
        &record.name,
        &record.description,
        &record.definition.nodes,
        record.version,
        record.is_built_in,
    ))
}

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct SaveTemplateArgs {
    pub name: String,
    pub description: String,
    pub nodes: Vec<nacc_domain::WorkflowNode>,
}

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct TemplateNameArgs {
    pub name: String,
}

/// Delete a *custom* template. Built-ins refuse with a typed error: the
/// presets are part of what NACC ships.
#[tauri::command]
#[specta::specta]
pub async fn delete_workflow_template(
    args: TemplateNameArgs,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    state
        .storage
        .delete_workflow_template(&args.name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn start_workflow_run(
    args: StartRunArgs,
    state: State<'_, AppState>,
) -> Result<RunSnapshotView, String> {
    // Stored templates first (custom graphs and synced built-ins); the
    // code-defined preset lookup is the fallback for a database that has not
    // received the startup sync yet. Both names share one namespace.
    let template = match state
        .storage
        .get_workflow_template(&args.template_name)
        .await
        .map_err(|e| e.to_string())?
    {
        Some(record) => record.definition,
        None => built_in_template(&args.template_name).ok_or_else(|| {
            let available: Vec<String> = built_in_templates()
                .into_iter()
                .map(|template| template.name)
                .collect();
            format!(
                "unknown workflow template `{}`; available: {}",
                args.template_name,
                available.join(", ")
            )
        })?,
    };

    let workspace = crate::routing::RoleMatrixRouting::validate_workspace(std::path::Path::new(
        &args.workspace,
    ))?;
    state
        .routing
        .set_workspace(args.project_id, workspace.clone());

    let snapshot = state
        .engine
        .start_run(args.project_id, &template)
        .await
        .map_err(|e| e.to_string())?;

    // Worktree isolation (master plan S16): when the user named a worktrees
    // root, the whole run works inside a freshly leased worktree and the
    // primary checkout is never touched. A failed allocation cancels the
    // just-created run with the reason instead of silently falling back to
    // the primary checkout -- an agent writing where the user did not choose
    // is exactly the failure mode this refuses.
    if let Some(root) = args
        .worktrees_root
        .as_deref()
        .map(str::trim)
        .filter(|root| !root.is_empty())
    {
        let root =
            crate::routing::RoleMatrixRouting::validate_workspace(std::path::Path::new(root))?;
        match allocate_run_worktree(&state, &snapshot, &template.name, &workspace, &root).await {
            Ok(lease) => {
                // The lease knows its path; routing hands it to every node of
                // this run until the lease is released.
                state
                    .routing
                    .set_run_workspace(snapshot.run.id, lease.path.clone());
                state
                    .run_leases
                    .lock()
                    .map_err(|_| {
                        "run lease registry is unavailable after an internal failure".to_string()
                    })?
                    .insert(snapshot.run.id, lease);
            }
            Err(err) => {
                let reason = format!("worktree allocation failed: {err}");
                let _ = state.engine.cancel(snapshot.run.id, &reason).await;
                return Err(reason);
            }
        }
    }

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
    let snapshot = state
        .engine
        .cancel(args.run_id, &args.reason)
        .await
        .map_err(|e| e.to_string())?;
    // The engine's cancel already stopped the in-flight provider sessions
    // through the executor; ending the run also ends its claim on its
    // worktree, so the lease is released now rather than waiting for a
    // session teardown that will not come. Dirty trees are quarantined by
    // the release policy, never destroyed.
    release_run_lease(
        &state.worktrees,
        &state.run_leases,
        &state.routing,
        args.run_id,
    )
    .await;
    Ok(snapshot_view(&snapshot))
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
    fn every_built_in_template_maps_to_a_runnable_view() {
        // The command itself reads storage (async, needs a running app), so
        // this checks the mapping the command performs over the code-defined
        // presets: the source of truth the startup sync writes from.
        let views: Vec<WorkflowTemplateView> = built_in_templates()
            .iter()
            .map(|template| {
                template_view(
                    &template.name,
                    &template.description,
                    &template.nodes,
                    1,
                    true,
                )
            })
            .collect();
        assert_eq!(views.len(), 6, "all six S18 presets must ship");
        for view in &views {
            assert!(
                !view.node_keys.is_empty(),
                "{} has no nodes, so it cannot be run",
                view.name
            );
            assert!(view.is_built_in);
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

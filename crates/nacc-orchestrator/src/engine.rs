//! The durable DAG engine (master plan S14.1/S14.3/S14.5, S16).
//!
//! # The shape of it
//!
//! The engine does not hold a run in memory. Every step is: read the durable
//! state, decide the next legal transition from that state, write what you are
//! about to do, do it, write what happened. Nothing is remembered that is not
//! also in SQLite, so a crash at any point leaves a state that
//! [`crate::recovery`] can explain and [`WorkflowEngine::resume`] can
//! continue -- which is the only reason to call a workflow "durable" rather
//! than "long-running".
//!
//! # What the engine deliberately does not do
//!
//! - It does not talk to a provider CLI. That is [`NodeExecutor`], injected:
//!   the engine owns *when* work happens and what is recorded, the executor
//!   owns *how* an agent is launched (argv, stream parsing, worktree lease).
//!   This is also what makes the whole engine testable without a single agent
//!   installed.
//! - It does not decide which provider or permission a role gets. That is
//!   [`RoleRouting`], which the Role Matrix (master plan S11) implements over
//!   real configuration rows.
//! - It does not auto-approve anything. A node that declares
//!   `requires_approval` stops the run until a human answers
//!   ([`WorkflowEngine::decide_approval`]); a rejection ends the run and the
//!   node never executes.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use nacc_domain::{
    ApprovalDecision, ApprovalId, AttemptId, AttemptTrigger, ModelId, NodeRunId, NodeState,
    PermissionProfile, ProjectId, ProviderId, RoleKind, RunState, WorkflowNode, WorkflowRunId,
    WorkflowTemplate,
};
use nacc_storage::{
    ApprovalRecord, CheckpointRecord, Database, NodeAttemptRecord, NodeRunRecord, WorkflowRunRecord,
};
use tokio::sync::Notify;
use tokio::task::JoinSet;

use crate::clock::{Clock, SystemClock};
use crate::governor::{Capacity, ConcurrencyLimits, Governor, Permit, SlotKey};
use crate::scheduler::{self, NodeStates, Outcome};
use crate::{OrchestratorError, Result};

/// How a node's failure is treated.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    /// Attempts per node, including the first. `1` disables retrying.
    pub max_attempts: u32,
    /// Backoff before attempt 2; doubles per further attempt.
    pub base_backoff: Duration,
    /// Ceiling for the doubling, so a long-lived run cannot end up scheduling
    /// an attempt for tomorrow.
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_backoff: Duration::from_secs(5),
            max_backoff: Duration::from_secs(60),
        }
    }
}

impl RetryPolicy {
    /// Exponential backoff, `base * 2^(failed_attempts - 1)`, capped.
    pub fn backoff_after(&self, failed_attempt: u32) -> Duration {
        let exponent = failed_attempt.saturating_sub(1).min(16);
        self.base_backoff
            .saturating_mul(1u32 << exponent)
            .min(self.max_backoff)
    }
}

/// Everything the engine needs beyond its collaborators.
#[derive(Copy, Clone, Debug)]
pub struct EngineConfig {
    pub limits: ConcurrencyLimits,
    pub retry: RetryPolicy,
    /// How long to wait for a concurrency slot that another run is holding
    /// before re-checking. Bounded polling rather than an unbounded wait,
    /// because a missed wake-up would otherwise stall a run forever.
    pub slot_wait_millis: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            limits: ConcurrencyLimits::default(),
            retry: RetryPolicy::default(),
            slot_wait_millis: 250,
        }
    }
}

/// One node's instruction, resolved and ready to hand to an adapter.
#[derive(Clone, Debug)]
pub struct NodeExecutionRequest {
    pub project_id: ProjectId,
    pub run_id: WorkflowRunId,
    pub node_run_id: NodeRunId,
    pub attempt_id: AttemptId,
    pub attempt_number: u32,
    pub trigger: AttemptTrigger,
    pub node_key: String,
    pub title: String,
    pub role: RoleKind,
    pub instruction: String,
    /// The effective profile: the node's declaration, narrowed by the role's
    /// configured profile. An executor must refuse anything outside it.
    pub permission_profile: PermissionProfile,
    pub provider_id: Option<ProviderId>,
    pub model_id: Option<ModelId>,
    /// Where the agent should work (a leased worktree, master plan S16). The
    /// executor allocates and releases it; the engine only carries it.
    pub workspace: Option<PathBuf>,
    /// The node's declared ceiling for one attempt. `None` lets the executor
    /// apply its default; a declared value is a per-node product decision,
    /// not a global guess (master plan S14's node contracts).
    pub timeout: Option<Duration>,
    /// Why a declared fallback was taken, when one was.
    pub fallback_reason: Option<String>,
}

/// What a successful attempt produced.
#[derive(Clone, Debug)]
pub struct NodeExecutionOutcome {
    pub summary: String,
}

/// Why an attempt failed. `retryable` is the executor's judgement about
/// *this* failure (a timeout or a rate limit is retryable; a rejected
/// instruction or a missing binary usually is not) and is combined with the
/// node's own `retryable` flag and the retry policy.
#[derive(Clone, Debug)]
pub struct NodeExecutionFailure {
    pub detail: String,
    pub retryable: bool,
}

impl NodeExecutionFailure {
    pub fn retryable(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            retryable: true,
        }
    }

    pub fn permanent(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            retryable: false,
        }
    }
}

/// Runs one workflow node. Implemented by the provider adapters (Claude Code,
/// Codex, ...); faked in tests.
#[async_trait]
pub trait NodeExecutor: Send + Sync {
    async fn execute(
        &self,
        request: NodeExecutionRequest,
    ) -> std::result::Result<NodeExecutionOutcome, NodeExecutionFailure>;

    /// Best-effort abort of one in-flight attempt. `false` means "this
    /// executor cannot cancel (or the attempt is already gone)" -- the
    /// default, so simple executors stay valid. The production executor
    /// kills the provider session's whole process tree through the adapter
    /// (master plan S13.4): without this, "cancelled" would be a durable
    /// state while the agent keeps running to completion.
    async fn cancel_attempt(&self, _run_id: WorkflowRunId, _attempt_id: AttemptId) -> bool {
        false
    }
}

/// Resolves a role to a provider, a model, a permission ceiling, and a
/// workspace. Implemented over the Role Matrix rows in the app; a fixed map
/// in tests.
pub trait RoleRouting: Send + Sync {
    /// `None` means the role is unassigned -- never "pick something".
    fn provider_for(&self, role: &RoleKind) -> Option<ProviderId>;

    fn model_for(&self, _role: &RoleKind) -> Option<ModelId> {
        None
    }

    /// A ceiling the role may not exceed. `None` means the node's own
    /// declared profile stands.
    fn permission_profile_for(&self, _role: &RoleKind) -> Option<PermissionProfile> {
        None
    }

    fn workspace_for(
        &self,
        _project_id: ProjectId,
        _run_id: WorkflowRunId,
        _node_key: &str,
    ) -> Option<PathBuf> {
        None
    }
}

/// A routing table fixed at construction: what a configured Role Matrix row
/// reduces to, without the database.
#[derive(Clone, Debug, Default)]
pub struct StaticRouting {
    rows: HashMap<String, RoleAssignment>,
}

#[derive(Clone, Debug, Default)]
pub struct RoleAssignment {
    pub provider_id: Option<ProviderId>,
    pub model_id: Option<ModelId>,
    pub permission_profile: Option<PermissionProfile>,
}

/// A stable, machine-readable key for a role. Roles are not `Hash` in the
/// domain (they are user-extensible and displayed verbatim), so the routing
/// table keys on their serialized form -- the same spelling that reaches
/// storage and the GUI.
pub fn role_key(role: &RoleKind) -> String {
    serde_json::to_string(role).unwrap_or_else(|_| format!("{role:?}"))
}

impl StaticRouting {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, role: RoleKind, assignment: RoleAssignment) -> Self {
        self.rows.insert(role_key(&role), assignment);
        self
    }

    /// Assign `role` to `provider` with no model override.
    pub fn assign(self, role: RoleKind, provider: ProviderId) -> Self {
        self.with(
            role,
            RoleAssignment {
                provider_id: Some(provider),
                ..RoleAssignment::default()
            },
        )
    }

    /// Assign every role in the list to one provider (the test convenience
    /// that keeps a fixture from listing every role).
    pub fn assign_all(mut self, roles: &[RoleKind], provider: ProviderId) -> Self {
        for role in roles {
            self = self.assign(role.clone(), provider);
        }
        self
    }

    pub fn row(&self, role: &RoleKind) -> Option<&RoleAssignment> {
        self.rows.get(&role_key(role))
    }
}

impl RoleRouting for StaticRouting {
    fn provider_for(&self, role: &RoleKind) -> Option<ProviderId> {
        self.row(role).and_then(|row| row.provider_id)
    }

    fn model_for(&self, role: &RoleKind) -> Option<ModelId> {
        self.row(role).and_then(|row| row.model_id.clone())
    }

    fn permission_profile_for(&self, role: &RoleKind) -> Option<PermissionProfile> {
        self.row(role).and_then(|row| row.permission_profile)
    }
}

/// Everything durable about one run, read at one point in time.
#[derive(Clone, Debug)]
pub struct RunSnapshot {
    pub run: WorkflowRunRecord,
    pub nodes: Vec<NodeRunRecord>,
    pub attempts: Vec<NodeAttemptRecord>,
    pub approvals: Vec<ApprovalRecord>,
    pub checkpoints: Vec<CheckpointRecord>,
}

impl RunSnapshot {
    pub fn node(&self, key: &str) -> Option<&NodeRunRecord> {
        self.nodes.iter().find(|node| node.node_key == key)
    }

    pub fn node_states(&self) -> NodeStates {
        self.nodes
            .iter()
            .map(|node| (node.node_key.clone(), node.state))
            .collect()
    }

    /// The graph as it was instantiated, taken from the persisted node rows
    /// rather than the current template -- a resumed run must follow the
    /// graph it started with.
    pub fn node_definitions(&self) -> Vec<WorkflowNode> {
        self.nodes
            .iter()
            .map(|node| node.definition.clone())
            .collect()
    }

    pub fn is_finished(&self) -> bool {
        self.run.state.is_terminal()
    }

    pub fn pending_approvals(&self) -> Vec<&ApprovalRecord> {
        self.approvals
            .iter()
            .filter(|approval| approval.decision.is_none())
            .collect()
    }

    pub fn pending_approval_for(&self, node_run_id: NodeRunId) -> Option<&ApprovalRecord> {
        self.approvals
            .iter()
            .find(|approval| approval.node_run_id == node_run_id && approval.decision.is_none())
    }

    /// Whether this node has already been approved. Only approvals recorded
    /// after the fact count: a rejection is not an approval, and a pending
    /// request is not either.
    pub fn is_approved(&self, node_key: &str) -> bool {
        let Some(node) = self.node(node_key) else {
            return false;
        };
        self.approvals.iter().any(|approval| {
            approval.node_run_id == node.id
                && matches!(approval.decision, Some(ApprovalDecision::Approved { .. }))
        })
    }

    pub fn is_rejected(&self, node_key: &str) -> bool {
        let Some(node) = self.node(node_key) else {
            return false;
        };
        self.approvals.iter().any(|approval| {
            approval.node_run_id == node.id
                && matches!(approval.decision, Some(ApprovalDecision::Rejected { .. }))
        })
    }

    /// Attempts that were started and never finished -- the fingerprint of an
    /// abnormal exit.
    pub fn unfinished_attempts(&self) -> Vec<&NodeAttemptRecord> {
        self.attempts
            .iter()
            .filter(|attempt| attempt.finished_state.is_none())
            .collect()
    }
}

/// Durable state transitions, in one place so that "every transition is
/// checkpointed" is a property of the code rather than a habit.
#[derive(Clone)]
struct Persistence {
    db: Arc<Database>,
    clock: Arc<dyn Clock>,
}

impl Persistence {
    async fn set_run_state(
        &self,
        run_id: WorkflowRunId,
        state: RunState,
        note: Option<String>,
        detail: &str,
    ) -> Result<()> {
        let mut run = self
            .db
            .get_workflow_run(run_id)
            .await?
            .ok_or(OrchestratorError::UnknownRun { run_id })?;
        run.state = state;
        run.note = note;
        run.updated_at_millis = self.clock.now_millis();
        self.db.update_workflow_run(&run).await?;
        self.db
            .append_checkpoint(run_id, state, detail, run.updated_at_millis)
            .await?;
        Ok(())
    }

    async fn set_node_state(
        &self,
        node: &NodeRunRecord,
        state: NodeState,
        detail: Option<String>,
    ) -> Result<()> {
        let mut updated = node.clone();
        updated.state = state;
        updated.last_detail = detail;
        updated.updated_at_millis = self.clock.now_millis();
        self.db.update_node_run(&updated).await?;
        Ok(())
    }
}

/// What one dispatch window did.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
struct DispatchReport {
    spawned: usize,
    refused: Option<Capacity>,
}

/// The engine. Cheap to clone in the sense that matters: share it through an
/// `Arc` and every caller drives the same durable state.
pub struct WorkflowEngine {
    db: Arc<Database>,
    executor: Arc<dyn NodeExecutor>,
    routing: Arc<dyn RoleRouting>,
    clock: Arc<dyn Clock>,
    config: EngineConfig,
    governor: Mutex<Governor>,
    /// Node key -> earliest millis at which its next attempt may start.
    /// In-memory on purpose: backoff is a scheduling nicety, and a restart
    /// that shortens it is harmless (`AttemptTrigger::Recovery` still records
    /// that the attempt was a recovery, so the audit trail does not lie).
    /// Shared with the attempt tasks through an `Arc` so a task can clear its
    /// own cooldown on success.
    cooldowns: Arc<Mutex<HashMap<String, u64>>>,
    /// Signalled whenever a concurrency slot is released, so a dispatch that
    /// found every slot taken can wake promptly instead of spinning.
    slot_freed: Notify,
    /// Attempt id -> owning run id, for exactly the attempts currently
    /// executing. This is the engine's half of cancellation: `cancel` uses it
    /// to reach the executor and stop the in-flight provider sessions, so a
    /// cancelled run does not keep burning tokens to a result nobody will
    /// read. In-memory by nature -- an attempt that survives a crash is what
    /// recovery reconciles, not what this map tracks.
    in_flight_attempts: Arc<Mutex<HashMap<AttemptId, WorkflowRunId>>>,
}

impl WorkflowEngine {
    pub fn new(
        db: Arc<Database>,
        executor: Arc<dyn NodeExecutor>,
        routing: Arc<dyn RoleRouting>,
        clock: Arc<dyn Clock>,
        config: EngineConfig,
    ) -> Self {
        Self {
            db,
            executor,
            routing,
            clock,
            config,
            governor: Mutex::new(Governor::new(config.limits)),
            cooldowns: Arc::new(Mutex::new(HashMap::new())),
            slot_freed: Notify::new(),
            in_flight_attempts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The common case: a real clock, the caller's executor and routing.
    pub fn with_defaults(
        db: Arc<Database>,
        executor: Arc<dyn NodeExecutor>,
        routing: Arc<dyn RoleRouting>,
    ) -> Self {
        Self::new(
            db,
            executor,
            routing,
            Arc::new(SystemClock),
            EngineConfig::default(),
        )
    }

    fn persistence(&self) -> Persistence {
        Persistence {
            db: self.db.clone(),
            clock: self.clock.clone(),
        }
    }

    /// How many nodes this engine is running right now (across every run it
    /// owns).
    pub fn in_flight(&self) -> usize {
        self.governor.lock().expect("governor poisoned").in_flight()
    }

    pub fn limits(&self) -> ConcurrencyLimits {
        self.governor.lock().expect("governor poisoned").limits()
    }

    // --- lifecycle ----------------------------------------------------

    /// Persist a run and its nodes, in `Pending`, without executing
    /// anything. Splitting "create" from "run" is what lets the GUI show a
    /// run before it starts, and what lets `start_and_run` be a two-liner.
    pub async fn start_run(
        &self,
        project_id: ProjectId,
        template: &WorkflowTemplate,
    ) -> Result<RunSnapshot> {
        scheduler::validate(&template.nodes)?;
        let now = self.clock.now_millis();
        let run = WorkflowRunRecord {
            id: WorkflowRunId::new(),
            project_id,
            template_name: template.name.clone(),
            state: RunState::Pending,
            note: None,
            created_at_millis: now,
            updated_at_millis: now,
        };
        self.db.insert_workflow_run(&run).await?;
        for node in &template.nodes {
            self.db
                .insert_node_run(&NodeRunRecord {
                    id: NodeRunId::new(),
                    workflow_run_id: run.id,
                    node_key: node.key.clone(),
                    title: node.title.clone(),
                    state: NodeState::Pending,
                    attempts: 0,
                    definition: node.clone(),
                    last_detail: None,
                    created_at_millis: now,
                    updated_at_millis: now,
                })
                .await?;
        }
        self.db
            .append_checkpoint(
                run.id,
                RunState::Pending,
                &format!("run created from template `{}`", template.name),
                now,
            )
            .await?;
        self.snapshot(run.id).await
    }

    pub async fn start_and_run(
        &self,
        project_id: ProjectId,
        template: &WorkflowTemplate,
    ) -> Result<RunSnapshot> {
        let snapshot = self.start_run(project_id, template).await?;
        self.run(snapshot.run.id).await
    }

    /// Drive a run until it reaches a terminal state or a stopping state
    /// (`Paused`, `AwaitingApproval`, `Interrupted`).
    pub async fn run(&self, run_id: WorkflowRunId) -> Result<RunSnapshot> {
        let snapshot = self.snapshot(run_id).await?;
        if snapshot.is_finished() {
            return Ok(snapshot);
        }
        if matches!(
            snapshot.run.state,
            RunState::Paused | RunState::AwaitingApproval | RunState::Interrupted
        ) {
            // Not an error: the caller asked to run something that is
            // deliberately stopped, and the answer is its current state.
            return Ok(snapshot);
        }
        self.persistence()
            .set_run_state(run_id, RunState::Running, None, "engine started")
            .await?;
        self.drive(run_id).await
    }

    /// Continue a run that was paused, interrupted, or left awaiting an
    /// approval that has since been decided.
    pub async fn resume(&self, run_id: WorkflowRunId) -> Result<RunSnapshot> {
        let snapshot = self.snapshot(run_id).await?;
        if snapshot.is_finished() {
            return Ok(snapshot);
        }
        if !snapshot.run.state.is_resumable() && snapshot.run.state != RunState::AwaitingApproval {
            return Err(OrchestratorError::NotResumable {
                run_id,
                state: snapshot.run.state,
            });
        }
        if snapshot.run.state == RunState::AwaitingApproval {
            // Resuming while the gate is still open would be an auto-approval
            // by accident, which master plan S12.2 forbids. Report the
            // current state instead.
            if snapshot.nodes.iter().any(|node| {
                node.definition.requires_approval && !snapshot.is_approved(&node.node_key)
            }) {
                return Ok(snapshot);
            }
        }
        let requeued = self.requeue_unfinished(run_id).await?;
        self.persistence()
            .set_run_state(
                run_id,
                RunState::Running,
                None,
                &format!("resumed; {requeued} node(s) re-queued"),
            )
            .await?;
        self.drive(run_id).await
    }

    /// Stop a run at a node boundary. Nodes already running are left alone:
    /// killing an agent mid-edit is not a "pause", and the worktree lease
    /// reconciliation (master plan S16) is what handles an agent that must
    /// actually be stopped.
    pub async fn pause(&self, run_id: WorkflowRunId, reason: &str) -> Result<RunSnapshot> {
        let snapshot = self.snapshot(run_id).await?;
        if snapshot.is_finished() {
            return Ok(snapshot);
        }
        self.persistence()
            .set_run_state(
                run_id,
                RunState::Paused,
                Some(reason.to_string()),
                &format!("paused: {reason}"),
            )
            .await?;
        self.snapshot(run_id).await
    }

    /// End a run and everything still pending in it. Terminal: a cancelled
    /// run is never resumed, because "cancelled" is a decision about the run
    /// rather than a pause in it.
    pub async fn cancel(&self, run_id: WorkflowRunId, reason: &str) -> Result<RunSnapshot> {
        let snapshot = self.snapshot(run_id).await?;
        if snapshot.is_finished() {
            return Ok(snapshot);
        }
        for node in &snapshot.nodes {
            if node.state.is_terminal() {
                continue;
            }
            self.persistence()
                .set_node_state(
                    node,
                    NodeState::Cancelled,
                    Some(format!("run cancelled: {reason}")),
                )
                .await?;
        }
        self.persistence()
            .set_run_state(
                run_id,
                RunState::Cancelled,
                Some(reason.to_string()),
                &format!("cancelled: {reason}"),
            )
            .await?;

        // The durable state now says cancelled; make the world match it.
        // Best-effort per attempt -- an executor that cannot cancel, or an
        // attempt that already finished, simply reports that -- but a
        // production executor kills the session's whole process tree here,
        // which is what "cancel" has to mean for a desktop app (S13.4):
        // without this, a cancelled run would keep burning tokens toward a
        // result nobody will read.
        let running: Vec<AttemptId> = {
            let map = self
                .in_flight_attempts
                .lock()
                .expect("in-flight attempts poisoned");
            map.iter()
                .filter(|(_, rid)| **rid == run_id)
                .map(|(aid, _)| *aid)
                .collect()
        };
        for attempt_id in running {
            if !self.executor.cancel_attempt(run_id, attempt_id).await {
                tracing::warn!(
                    run = %run_id,
                    attempt = %attempt_id,
                    "executor could not cancel an in-flight attempt"
                );
            }
        }

        self.snapshot(run_id).await
    }

    /// Record a human's answer to an approval gate. Approving does not
    /// resume the run by itself; [`WorkflowEngine::approve_and_resume`] does
    /// both, so the two-step version stays available to a UI that wants to
    /// show the decision before continuing.
    pub async fn decide_approval(
        &self,
        run_id: WorkflowRunId,
        approval_id: ApprovalId,
        decision: &ApprovalDecision,
    ) -> Result<()> {
        self.db
            .decide_approval(approval_id, decision, self.clock.now_millis())
            .await?;
        let snapshot = self.snapshot(run_id).await?;
        match decision {
            ApprovalDecision::Approved { by } => {
                if let Some(approval) = snapshot
                    .approvals
                    .iter()
                    .find(|approval| approval.id == approval_id)
                {
                    if let Some(node) = snapshot
                        .nodes
                        .iter()
                        .find(|node| node.id == approval.node_run_id)
                    {
                        self.persistence()
                            .set_node_state(
                                node,
                                NodeState::Pending,
                                Some(format!("approved by {by}")),
                            )
                            .await?;
                        self.db
                            .append_checkpoint(
                                run_id,
                                RunState::AwaitingApproval,
                                &format!("`{}` approved by {by}", node.node_key),
                                self.clock.now_millis(),
                            )
                            .await?;
                    }
                }
                Ok(())
            }
            ApprovalDecision::Rejected { by, reason } => {
                // A rejection is final for the node and for the run: the
                // operation was refused, so continuing would either skip the
                // gated operation (silently weakening the workflow) or run it
                // anyway (which is what an approval gate exists to prevent).
                if let Some(approval) = snapshot
                    .approvals
                    .iter()
                    .find(|approval| approval.id == approval_id)
                {
                    if let Some(node) = snapshot
                        .nodes
                        .iter()
                        .find(|node| node.id == approval.node_run_id)
                    {
                        if !node.state.is_terminal() {
                            self.persistence()
                                .set_node_state(
                                    node,
                                    NodeState::Cancelled,
                                    Some(format!("rejected by {by}: {reason}")),
                                )
                                .await?;
                        }
                    }
                }
                self.persistence()
                    .set_run_state(
                        run_id,
                        RunState::Cancelled,
                        Some(format!("approval rejected by {by}: {reason}")),
                        &format!("run cancelled: approval rejected by {by}"),
                    )
                    .await?;
                Ok(())
            }
        }
    }

    /// The one-call version the GUI uses: approve, then continue.
    pub async fn approve_and_resume(
        &self,
        run_id: WorkflowRunId,
        approval_id: ApprovalId,
        by: &str,
    ) -> Result<RunSnapshot> {
        self.decide_approval(
            run_id,
            approval_id,
            &ApprovalDecision::Approved { by: by.to_string() },
        )
        .await?;
        self.resume(run_id).await
    }

    // --- reads --------------------------------------------------------

    pub async fn snapshot(&self, run_id: WorkflowRunId) -> Result<RunSnapshot> {
        let run = self
            .db
            .get_workflow_run(run_id)
            .await?
            .ok_or(OrchestratorError::UnknownRun { run_id })?;
        Ok(RunSnapshot {
            nodes: self.db.list_node_runs(run_id).await?,
            attempts: self.db.list_node_attempts(run_id).await?,
            approvals: self.db.list_approvals(run_id).await?,
            checkpoints: self.db.list_checkpoints(run_id).await?,
            run,
        })
    }

    /// Runs a restart has to explain: mid-flight, waiting on a human, or
    /// abandoned by a crash. The GUI's "needs attention" list.
    pub async fn runs_needing_attention(&self) -> Result<Vec<WorkflowRunRecord>> {
        Ok(self
            .db
            .list_workflow_runs_in_states(&[
                RunState::Running,
                RunState::AwaitingApproval,
                RunState::Interrupted,
            ])
            .await?)
    }

    /// Every run that has not reached a terminal state, including ones
    /// created but never started.
    pub async fn open_runs(&self) -> Result<Vec<WorkflowRunRecord>> {
        Ok(self
            .db
            .list_workflow_runs_in_states(&[
                RunState::Pending,
                RunState::Running,
                RunState::Paused,
                RunState::AwaitingApproval,
                RunState::Interrupted,
            ])
            .await?)
    }

    pub async fn runs_for_project(&self, project_id: ProjectId) -> Result<Vec<WorkflowRunRecord>> {
        Ok(self.db.list_workflow_runs_for_project(project_id).await?)
    }

    // --- the drive loop -----------------------------------------------

    async fn drive(&self, run_id: WorkflowRunId) -> Result<RunSnapshot> {
        loop {
            let snapshot = self.snapshot(run_id).await?;
            if snapshot.is_finished()
                || matches!(
                    snapshot.run.state,
                    RunState::Paused | RunState::AwaitingApproval | RunState::Interrupted
                )
            {
                return Ok(snapshot);
            }

            let nodes = snapshot.node_definitions();
            let states = snapshot.node_states();

            // 1. Nodes whose dependencies can never succeed are marked
            //    `Skipped` one layer per pass, so a deep cone propagates to
            //    the leaves instead of leaving orphans behind.
            let blocked = scheduler::newly_blocked(&nodes, &states);
            if !blocked.is_empty() {
                for node in blocked {
                    if let Some(record) = snapshot.node(&node.key) {
                        self.persistence()
                            .set_node_state(
                                record,
                                NodeState::Skipped,
                                Some("a dependency did not succeed".to_string()),
                            )
                            .await?;
                    }
                }
                continue;
            }

            let ready: Vec<&WorkflowNode> = scheduler::runnable(&nodes, &states);
            if ready.is_empty() {
                return self.finish(run_id, &nodes, &states).await;
            }

            // 2. An approval gate stops the run before anything else is
            //    decided about that node.
            if let Some(gated) = ready
                .iter()
                .copied()
                .find(|node| node.requires_approval && !snapshot.is_approved(&node.key))
            {
                return self.await_approval(run_id, &snapshot, gated).await;
            }

            // 3. Nodes waiting out a retry backoff are held back. If nothing
            //    else can run, the engine waits for the earliest of them
            //    rather than spinning.
            let now = self.clock.now_millis();
            let cooldowns = self.cooldowns.lock().expect("cooldowns poisoned").clone();
            let (dispatchable, cooling): (Vec<&WorkflowNode>, Vec<&WorkflowNode>) = ready
                .iter()
                .copied()
                .partition(|node| cooldowns.get(&node.key).is_none_or(|at| *at <= now));
            if dispatchable.is_empty() {
                let earliest = cooling
                    .iter()
                    .filter_map(|node| cooldowns.get(&node.key).copied())
                    .min()
                    .unwrap_or(now);
                self.clock
                    .sleep(Duration::from_millis(earliest.saturating_sub(now).max(1)))
                    .await;
                continue;
            }

            let report = self.dispatch(run_id, &snapshot, dispatchable).await?;
            if report.refused.is_some() {
                self.wait_for_slot().await;
            }
        }
    }

    /// Decide the run's fate once no node can run.
    async fn finish(
        &self,
        run_id: WorkflowRunId,
        nodes: &[WorkflowNode],
        states: &NodeStates,
    ) -> Result<RunSnapshot> {
        match scheduler::outcome(nodes, states) {
            Outcome::Succeeded => {
                self.persistence()
                    .set_run_state(run_id, RunState::Succeeded, None, "every node succeeded")
                    .await?;
            }
            Outcome::Failed => {
                let failures = scheduler::failures(nodes, states);
                self.persistence()
                    .set_run_state(
                        run_id,
                        RunState::Failed,
                        Some(format!("nodes did not succeed: {}", failures.join(", "))),
                        "run failed",
                    )
                    .await?;
            }
            Outcome::Running => {
                // No node is runnable and none is running, yet something is
                // still non-terminal: a state the engine cannot explain. It
                // is reported as interrupted with the reason, never as a
                // success or a hang.
                self.persistence()
                    .set_run_state(
                        run_id,
                        RunState::Interrupted,
                        Some("no node can make progress".to_string()),
                        "stalled: no node runnable and none running",
                    )
                    .await?;
            }
        }
        self.snapshot(run_id).await
    }

    async fn await_approval(
        &self,
        run_id: WorkflowRunId,
        snapshot: &RunSnapshot,
        node: &WorkflowNode,
    ) -> Result<RunSnapshot> {
        let Some(record) = snapshot.node(&node.key) else {
            return Err(OrchestratorError::NodeNotInRun {
                run_id,
                node_key: node.key.clone(),
            });
        };
        let now = self.clock.now_millis();
        if snapshot.pending_approval_for(record.id).is_none() {
            self.db
                .insert_approval(&ApprovalRecord {
                    id: ApprovalId::new(),
                    workflow_run_id: run_id,
                    node_run_id: record.id,
                    summary: format!(
                        "Approve `{}` ({}) under the {} profile: {}",
                        node.title, node.key, node.permission_profile_hint, node.instruction
                    ),
                    requested_at_millis: now,
                    decision: None,
                    decided_at_millis: None,
                })
                .await?;
        }
        self.persistence()
            .set_node_state(
                record,
                NodeState::Pending,
                Some("waiting for human approval".to_string()),
            )
            .await?;
        self.persistence()
            .set_run_state(
                run_id,
                RunState::AwaitingApproval,
                None,
                &format!("awaiting approval for `{}`", node.key),
            )
            .await?;
        self.snapshot(run_id).await
    }

    /// Re-queue nodes left `Running` by an abnormal exit, and close their
    /// orphaned attempt rows. Shared with startup recovery.
    pub async fn requeue_unfinished(&self, run_id: WorkflowRunId) -> Result<usize> {
        let snapshot = self.snapshot(run_id).await?;
        let mut requeued = 0;
        for node in snapshot
            .nodes
            .iter()
            .filter(|node| node.state == NodeState::Running)
        {
            self.persistence()
                .set_node_state(
                    node,
                    NodeState::Pending,
                    Some("re-queued after an interrupted attempt".to_string()),
                )
                .await?;
            requeued += 1;
        }
        for attempt in snapshot.unfinished_attempts() {
            self.db
                .finish_node_attempt(
                    attempt.id,
                    NodeState::Failed,
                    "NACC stopped before this attempt finished",
                    self.clock.now_millis(),
                )
                .await?;
        }
        // A re-queued node must not inherit a stale cooldown.
        let mut cooldowns = self.cooldowns.lock().expect("cooldowns poisoned");
        cooldowns.clear();
        Ok(requeued)
    }

    /// Record that an abnormal exit interrupted a run (startup recovery, and
    /// the app's own clean-shutdown path). The run becomes resumable rather
    /// than failed: nothing about the work so far is known to be wrong.
    pub async fn mark_interrupted(
        &self,
        run_id: WorkflowRunId,
        note: &str,
        requeued_nodes: usize,
        interrupted_attempts: usize,
    ) -> Result<()> {
        self.persistence()
            .set_run_state(
                run_id,
                RunState::Interrupted,
                Some(note.to_string()),
                &format!(
                    "interrupted: {note} ({requeued_nodes} node(s) re-queued, \
                     {interrupted_attempts} attempt(s) closed)"
                ),
            )
            .await
    }

    /// Run as many of `ready` as the governor admits, and wait for those to
    /// finish. Nodes are considered oldest-first, and the whole ready set is
    /// re-examined after each completion, which is the fairness rule the
    /// governor itself does not enforce.
    async fn dispatch(
        &self,
        run_id: WorkflowRunId,
        snapshot: &RunSnapshot,
        ready: Vec<&WorkflowNode>,
    ) -> Result<DispatchReport> {
        let mut queue: VecDeque<WorkflowNode> = ready.into_iter().cloned().collect();
        let mut permits: HashMap<String, Permit> = HashMap::new();
        let mut report = DispatchReport::default();
        let mut failure: Option<OrchestratorError> = None;
        let mut set: JoinSet<(String, Result<()>)> = JoinSet::new();

        loop {
            while let Some(node) = queue.front().cloned() {
                let persistence = self.persistence();
                let executor = self.executor.clone();
                let routing = self.routing.clone();
                let retry = self.config.retry;
                let cooldowns = self.cooldowns.clone();
                let in_flight = self.in_flight_attempts.clone();
                let run = snapshot.run.clone();
                let provider = self.routing.provider_for(&node.role);
                let key = SlotKey::new(snapshot.run.project_id, provider);
                let permit = {
                    let mut governor = self.governor.lock().expect("governor poisoned");
                    governor.try_acquire(key)
                };
                let Some(permit) = permit else {
                    report.refused = self
                        .governor
                        .lock()
                        .expect("governor poisoned")
                        .capacity_for(key);
                    break;
                };
                queue.pop_front();
                report.spawned += 1;
                let node_run_id = snapshot.node(&node.key).map(|record| record.id).ok_or(
                    OrchestratorError::NodeNotInRun {
                        run_id,
                        node_key: node.key.clone(),
                    },
                )?;
                let trigger = self.attempt_trigger(snapshot, node_run_id, &node.key);
                permits.insert(node.key.clone(), permit);
                // The task owns everything it touches and reports back by
                // node key, so the engine can release exactly that node's
                // concurrency slot when it lands.
                let task_key = node.key.clone();
                set.spawn(async move {
                    let result = run_attempt(
                        persistence,
                        executor,
                        routing,
                        retry,
                        cooldowns,
                        in_flight,
                        run,
                        node,
                        trigger,
                    )
                    .await;
                    (task_key, result)
                });
            }

            let Some(joined) = set.join_next().await else {
                break;
            };
            match joined {
                Ok((node_key, result)) => {
                    if let Some(permit) = permits.remove(&node_key) {
                        self.governor
                            .lock()
                            .expect("governor poisoned")
                            .release(permit);
                        self.slot_freed.notify_waiters();
                    }
                    if let Err(err) = result {
                        if failure.is_none() {
                            failure = Some(err);
                        }
                    }
                }
                Err(join_error) => {
                    // A panicking executor must not take the run down with it.
                    return Err(OrchestratorError::NodeTaskPanicked {
                        run_id,
                        detail: join_error.to_string(),
                    });
                }
            }
        }

        if let Some(err) = failure {
            // Everything that was spawned has been joined by now, and the
            // run is left `Running` with its per-node state accurately
            // recorded, so recovery can pick it up.
            return Err(err);
        }
        Ok(report)
    }

    fn attempt_trigger(
        &self,
        snapshot: &RunSnapshot,
        node_run_id: NodeRunId,
        node_key: &str,
    ) -> AttemptTrigger {
        let has_unfinished = snapshot
            .attempts
            .iter()
            .any(|attempt| attempt.node_run_id == node_run_id && attempt.finished_state.is_none());
        if has_unfinished {
            return AttemptTrigger::Recovery;
        }
        match snapshot.node(node_key).map(|node| node.attempts) {
            Some(0) | None => AttemptTrigger::Initial,
            Some(_) => {
                // A Pending node with past attempts is a retry only if the
                // engine itself scheduled one -- its cooldown entry is still
                // present until the node succeeds. Recovery requeues without
                // a cooldown, and so does a dispatch after a restart (the
                // cooldown map is in-memory by design), which is exactly the
                // situation master plan S14.5 wants recorded as Recovery.
                let scheduled_retry = self
                    .cooldowns
                    .lock()
                    .expect("cooldowns poisoned")
                    .contains_key(node_key);
                if scheduled_retry {
                    AttemptTrigger::Retry
                } else {
                    AttemptTrigger::Recovery
                }
            }
        }
    }

    async fn wait_for_slot(&self) {
        let waited = tokio::time::timeout(
            Duration::from_millis(self.config.slot_wait_millis.max(1)),
            self.slot_freed.notified(),
        )
        .await;
        if waited.is_err() {
            tracing::debug!("no concurrency slot freed within the poll window; re-checking");
        }
    }
}

/// One attempt at one node, start to finish. A free function so the task it
/// becomes owns everything it touches -- the engine is not borrowed across
/// the await.
#[allow(clippy::too_many_arguments)]
async fn run_attempt(
    persistence: Persistence,
    executor: Arc<dyn NodeExecutor>,
    routing: Arc<dyn RoleRouting>,
    retry: RetryPolicy,
    cooldowns: Arc<Mutex<HashMap<String, u64>>>,
    in_flight: Arc<Mutex<HashMap<AttemptId, WorkflowRunId>>>,
    run: WorkflowRunRecord,
    node: WorkflowNode,
    trigger: AttemptTrigger,
) -> Result<()> {
    let snapshot_node = persistence
        .db
        .list_node_runs(run.id)
        .await?
        .into_iter()
        .find(|record| record.node_key == node.key)
        .ok_or(OrchestratorError::NodeNotInRun {
            run_id: run.id,
            node_key: node.key.clone(),
        })?;

    let attempt_number = snapshot_node.attempts + 1;
    let attempt_id = AttemptId::new();

    // Resolve routing *before* recording the attempt, so the attempt row
    // names the provider that actually ran rather than the one that was
    // intended.
    let assigned = routing.provider_for(&node.role);
    let (provider_id, trigger, fallback_reason) = match assigned {
        Some(provider) => (Some(provider), trigger, None),
        None => match node.fallbacks.first() {
            Some(fallback) => (
                Some(fallback.provider_id),
                AttemptTrigger::Fallback,
                Some(fallback.reason.clone()),
            ),
            None => (None, trigger, None),
        },
    };
    let model_id = routing.model_for(&node.role);
    let declared = node.permission_profile_hint;
    let permission_profile = match routing.permission_profile_for(&node.role) {
        Some(role_ceiling) => declared.narrower_of(role_ceiling),
        None => declared,
    };
    let workspace = routing.workspace_for(run.project_id, run.id, &node.key);

    let mut running = snapshot_node.clone();
    running.state = NodeState::Running;
    running.attempts = attempt_number;
    running.last_detail = Some(format!("attempt {attempt_number} started"));
    running.updated_at_millis = persistence.clock.now_millis();
    persistence.db.update_node_run(&running).await?;
    persistence
        .db
        .insert_node_attempt(&NodeAttemptRecord {
            id: attempt_id,
            node_run_id: snapshot_node.id,
            workflow_run_id: run.id,
            attempt_number,
            trigger,
            finished_state: None,
            provider: provider_id,
            detail: fallback_reason.clone(),
            started_at_millis: running.updated_at_millis,
            finished_at_millis: None,
        })
        .await?;

    let Some(provider_id) = provider_id else {
        // No provider and no declared fallback: retrying cannot fix an
        // unconfigured role, so this fails now, visibly, with the reason.
        let detail = format!(
            "role {:?} has no provider assigned and node `{}` declares no fallback",
            node.role, node.key
        );
        persistence
            .db
            .finish_node_attempt(
                attempt_id,
                NodeState::Failed,
                &detail,
                persistence.clock.now_millis(),
            )
            .await?;
        let mut failed = running.clone();
        failed.state = NodeState::Failed;
        failed.last_detail = Some(detail);
        failed.updated_at_millis = persistence.clock.now_millis();
        persistence.db.update_node_run(&failed).await?;
        return Ok(());
    };

    let request = NodeExecutionRequest {
        project_id: run.project_id,
        run_id: run.id,
        node_run_id: snapshot_node.id,
        attempt_id,
        attempt_number,
        trigger,
        node_key: node.key.clone(),
        title: node.title.clone(),
        role: node.role.clone(),
        instruction: node.instruction.clone(),
        permission_profile,
        provider_id: Some(provider_id),
        model_id,
        workspace,
        timeout: node.timeout_secs.map(|secs| Duration::from_secs(u64::from(secs))),
        fallback_reason,
    };

    // Register the attempt as cancellable before launching and deregister on
    // the single exit path below, so `cancel` reaches exactly the live
    // sessions -- no window where a finishing attempt is still listed, no
    // attempt that cannot be reached.
    in_flight
        .lock()
        .expect("in-flight attempts poisoned")
        .insert(attempt_id, run.id);
    let outcome = executor.execute(request).await;
    in_flight
        .lock()
        .expect("in-flight attempts poisoned")
        .remove(&attempt_id);

    // A cancel that landed while the attempt was in flight already wrote the
    // durable node state. The late result must not overwrite it -- a cancelled
    // node must not come back as "failed" or "pending" because its orphaned
    // attempt finally reported. The attempt row still records what actually
    // happened; only the node-level state stays as the cancellation left it.
    let overtaken = matches!(
        persistence
            .db
            .list_node_runs(run.id)
            .await?
            .into_iter()
            .find(|record| record.id == snapshot_node.id)
            .map(|record| record.state),
        Some(NodeState::Cancelled) | Some(NodeState::Skipped)
    );

    match outcome {
        Ok(outcome) => {
            let finished_at = persistence.clock.now_millis();
            persistence
                .db
                .finish_node_attempt(
                    attempt_id,
                    NodeState::Succeeded,
                    &outcome.summary,
                    finished_at,
                )
                .await?;
            if overtaken {
                tracing::warn!(
                    run = %run.id,
                    node = %node.key,
                    "attempt succeeded after its run was cancelled; node state left as cancelled"
                );
                return Ok(());
            }
            let mut succeeded = running.clone();
            succeeded.state = NodeState::Succeeded;
            succeeded.last_detail = Some(outcome.summary);
            succeeded.updated_at_millis = finished_at;
            persistence.db.update_node_run(&succeeded).await?;
            cooldowns
                .lock()
                .expect("cooldowns poisoned")
                .remove(&node.key);
            Ok(())
        }
        Err(failure) => {
            let finished_at = persistence.clock.now_millis();
            persistence
                .db
                .finish_node_attempt(attempt_id, NodeState::Failed, &failure.detail, finished_at)
                .await?;
            if overtaken {
                tracing::warn!(
                    run = %run.id,
                    node = %node.key,
                    "attempt failed after its run was cancelled; node state left as cancelled"
                );
                return Ok(());
            }
            let attempts_exhausted = attempt_number >= retry.max_attempts;
            let will_retry = failure.retryable && node.retryable && !attempts_exhausted;
            let mut updated = running.clone();
            updated.updated_at_millis = finished_at;
            if will_retry {
                let backoff = retry.backoff_after(attempt_number);
                updated.state = NodeState::Pending;
                updated.last_detail = Some(format!(
                    "attempt {attempt_number} failed: {}; retrying in {}s",
                    failure.detail,
                    backoff.as_secs()
                ));
                cooldowns
                    .lock()
                    .expect("cooldowns poisoned")
                    .insert(node.key.clone(), finished_at + backoff.as_millis() as u64);
            } else {
                let why = if !node.retryable {
                    "the node is declared non-retryable"
                } else if !failure.retryable {
                    "the failure is not retryable"
                } else {
                    "attempts exhausted"
                };
                updated.state = NodeState::Failed;
                updated.last_detail = Some(format!(
                    "attempt {attempt_number} failed: {} ({why})",
                    failure.detail
                ));
                cooldowns
                    .lock()
                    .expect("cooldowns poisoned")
                    .remove(&node.key);
            }
            persistence.db.update_node_run(&updated).await?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;

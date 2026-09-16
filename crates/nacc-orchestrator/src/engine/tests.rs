//! Engine tests: the whole state machine, driven with a fake executor, a
//! deterministic clock, and a real (in-memory) SQLite database.
//!
//! The database is the real one on purpose. Most of what can go wrong in this
//! engine is a transition that is not persisted, a row that is written twice,
//! or an attempt whose history does not add up -- none of which a mocked
//! storage layer would catch.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nacc_domain::{
    AttemptTrigger, NodeFallback, NodeState, PermissionProfile, ProviderId, RoleKind, RunState,
    WorkflowNode, WorkflowTemplate,
};
use nacc_storage::Database;
use tokio::sync::Barrier;

use super::*;
use crate::clock::VirtualClock;
use crate::recovery;

const EXPLORER: RoleKind = RoleKind::RepositoryExplorer;
const IMPLEMENTER: RoleKind = RoleKind::BackendImplementer;
const TESTER: RoleKind = RoleKind::TestEngineer;
const INTEGRATOR: RoleKind = RoleKind::Integrator;

#[derive(Clone, Debug)]
enum Behaviour {
    Succeed(String),
    Fail { detail: String, retryable: bool },
}

impl Behaviour {
    fn ok() -> Self {
        Behaviour::Succeed("done".to_string())
    }

    fn retryable(detail: &str) -> Self {
        Behaviour::Fail {
            detail: detail.to_string(),
            retryable: true,
        }
    }

    fn permanent(detail: &str) -> Self {
        Behaviour::Fail {
            detail: detail.to_string(),
            retryable: false,
        }
    }
}

/// An executor that records every request, can be scripted per node, and can
/// be made to prove that two nodes really ran at the same time.
#[derive(Clone, Default)]
struct FakeExecutor {
    seen: Arc<Mutex<Vec<NodeExecutionRequest>>>,
    scripted: Arc<Mutex<HashMap<String, VecDeque<Behaviour>>>>,
    live: Arc<AtomicUsize>,
    peak_live: Arc<AtomicUsize>,
    gate: Option<Arc<Barrier>>,
    /// Which node keys the gate applies to. The overlap proof must gate only
    /// the nodes that are supposed to coexist: a later dependent node runs
    /// alone, and gating it too would fail the run for reasons the engine
    /// does not control.
    gate_keys: Vec<String>,
    delay: Option<Duration>,
    panic_on: Option<String>,
    fail_to_launch: bool,
}

impl FakeExecutor {
    fn new() -> Self {
        Self::default()
    }

    fn script(self, node_key: &str, behaviours: Vec<Behaviour>) -> Self {
        self.scripted
            .lock()
            .expect("scripted poisoned")
            .insert(node_key.to_string(), behaviours.into());
        self
    }

    /// Require the named `keys` nodes to be running simultaneously, or fail
    /// the attempt. This is how "independent nodes actually overlap" is
    /// proved rather than assumed.
    fn requiring_overlap(mut self, parties: usize, keys: &[&str]) -> Self {
        self.gate = Some(Arc::new(Barrier::new(parties)));
        self.gate_keys = keys.iter().map(|key| key.to_string()).collect();
        self
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    fn panicking_on(mut self, node_key: &str) -> Self {
        self.panic_on = Some(node_key.to_string());
        self
    }

    /// Every launch fails as if the provider CLI were missing.
    fn with_missing_provider(mut self) -> Self {
        self.fail_to_launch = true;
        self
    }

    fn peak_concurrency(&self) -> usize {
        self.peak_live.load(Ordering::SeqCst)
    }

    fn executed(&self, node_key: &str) -> bool {
        self.seen
            .lock()
            .expect("seen poisoned")
            .iter()
            .any(|request| request.node_key == node_key)
    }

    fn requests(&self, node_key: &str) -> Vec<NodeExecutionRequest> {
        self.seen
            .lock()
            .expect("seen poisoned")
            .iter()
            .filter(|request| request.node_key == node_key)
            .cloned()
            .collect()
    }

    fn order(&self) -> Vec<String> {
        self.seen
            .lock()
            .expect("seen poisoned")
            .iter()
            .map(|request| request.node_key.clone())
            .collect()
    }
}

#[async_trait]
impl NodeExecutor for FakeExecutor {
    async fn execute(
        &self,
        request: NodeExecutionRequest,
    ) -> std::result::Result<NodeExecutionOutcome, NodeExecutionFailure> {
        let live = self.live.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak_live.fetch_max(live, Ordering::SeqCst);
        self.seen
            .lock()
            .expect("seen poisoned")
            .push(request.clone());

        if let Some(gate) = &self.gate {
            if self.gate_keys.iter().any(|key| *key == request.node_key)
                && tokio::time::timeout(Duration::from_secs(10), gate.wait())
                    .await
                    .is_err()
            {
                self.live.fetch_sub(1, Ordering::SeqCst);
                return Err(NodeExecutionFailure::permanent(
                    "this node's peer never started: the engine did not overlap them",
                ));
            }
        }
        if self.fail_to_launch {
            self.live.fetch_sub(1, Ordering::SeqCst);
            return Err(NodeExecutionFailure::retryable(
                "provider CLI not found on PATH",
            ));
        }
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        self.live.fetch_sub(1, Ordering::SeqCst);

        if self.panic_on.as_deref() == Some(request.node_key.as_str()) {
            panic!("executor panicked for {}", request.node_key);
        }

        let behaviour = self
            .scripted
            .lock()
            .expect("scripted poisoned")
            .get_mut(&request.node_key)
            .and_then(|queue| queue.pop_front())
            .unwrap_or_else(Behaviour::ok);
        match behaviour {
            Behaviour::Succeed(summary) => Ok(NodeExecutionOutcome { summary }),
            Behaviour::Fail { detail, retryable } => {
                Err(NodeExecutionFailure { detail, retryable })
            }
        }
    }
}

fn node(key: &str, role: RoleKind, depends_on: &[&str]) -> WorkflowNode {
    WorkflowNode {
        key: key.to_string(),
        title: format!("{key} title"),
        role,
        depends_on: depends_on.iter().map(|dep| dep.to_string()).collect(),
        instruction: format!("do the work for {key} carefully and report exactly what happened"),
        permission_profile_hint: PermissionProfile::AutonomousWorktree,
        retryable: true,
        requires_approval: false,
        fallbacks: vec![],
    }
}

fn gated(mut node: WorkflowNode) -> WorkflowNode {
    node.requires_approval = true;
    node.permission_profile_hint = PermissionProfile::RepositoryMaintainer;
    node
}

fn template(name: &str, nodes: Vec<WorkflowNode>) -> WorkflowTemplate {
    WorkflowTemplate {
        name: name.to_string(),
        description: "test template".to_string(),
        nodes,
    }
}

fn all_roles_claude() -> StaticRouting {
    StaticRouting::new().assign_all(
        &[EXPLORER, IMPLEMENTER, TESTER, INTEGRATOR],
        ProviderId::Claude,
    )
}

struct Harness {
    db: Arc<Database>,
    clock: Arc<VirtualClock>,
    engine: WorkflowEngine,
}

fn harness(executor: FakeExecutor, routing: StaticRouting, config: EngineConfig) -> Harness {
    let db = Arc::new(Database::open_in_memory().expect("in-memory database"));
    let clock = Arc::new(VirtualClock::starting_at(1_000));
    let engine = WorkflowEngine::new(
        db.clone(),
        Arc::new(executor),
        Arc::new(routing),
        clock.clone(),
        config,
    );
    Harness { db, clock, engine }
}

fn harness_with(executor: FakeExecutor, routing: StaticRouting) -> Harness {
    harness(executor, routing, EngineConfig::default())
}

fn fast_config() -> EngineConfig {
    EngineConfig {
        retry: RetryPolicy {
            max_attempts: 3,
            base_backoff: Duration::from_secs(5),
            max_backoff: Duration::from_secs(60),
        },
        ..EngineConfig::default()
    }
}

fn state_of(snapshot: &RunSnapshot, key: &str) -> NodeState {
    snapshot
        .node(key)
        .unwrap_or_else(|| panic!("node {key} missing from the snapshot"))
        .state
}

// --- happy paths --------------------------------------------------------

#[tokio::test]
async fn a_linear_run_executes_every_node_in_order_and_succeeds() {
    let executor = FakeExecutor::new();
    let harness = harness_with(executor.clone(), all_roles_claude());
    let snapshot = harness
        .engine
        .start_and_run(
            ProjectId::new(),
            &template(
                "linear",
                vec![
                    node("explore", EXPLORER, &[]),
                    node("implement", IMPLEMENTER, &["explore"]),
                    node("verify", TESTER, &["implement"]),
                ],
            ),
        )
        .await
        .expect("run should complete");

    assert_eq!(snapshot.run.state, RunState::Succeeded);
    for key in ["explore", "implement", "verify"] {
        assert_eq!(state_of(&snapshot, key), NodeState::Succeeded, "{key}");
    }
    // Every node produced its own attempt row, and each one finished.
    assert_eq!(snapshot.attempts.len(), 3);
    assert!(snapshot.attempts.iter().all(|attempt| {
        attempt.finished_state == Some(NodeState::Succeeded) && attempt.finished_at_millis.is_some()
    }));
    assert_eq!(
        snapshot
            .checkpoints
            .first()
            .map(|checkpoint| checkpoint.state),
        Some(RunState::Pending)
    );
    // A chain of dependencies is not a fan-out: each node waits for the one
    // before it, so the executor sees exactly this order.
    assert_eq!(executor.order(), vec!["explore", "implement", "verify"]);
    assert_eq!(
        snapshot
            .checkpoints
            .last()
            .map(|checkpoint| checkpoint.state),
        Some(RunState::Succeeded)
    );
}

#[tokio::test]
async fn independent_nodes_really_overlap() {
    // The enterprise-feature preset exists for its parallel exploration. If
    // the engine serialized independent nodes, the run would still "pass" --
    // so the executor blocks until both explorer nodes are inside it at once.
    // Only the explorers are gated: `contract` depends on them and by design
    // runs alone afterwards.
    let executor =
        FakeExecutor::new().requiring_overlap(2, &["explore_frontend", "explore_backend"]);
    let harness = harness_with(executor, all_roles_claude());
    let snapshot = harness
        .engine
        .start_and_run(
            ProjectId::new(),
            &template(
                "parallel",
                vec![
                    node("explore_frontend", EXPLORER, &[]),
                    node("explore_backend", EXPLORER, &[]),
                    node(
                        "contract",
                        IMPLEMENTER,
                        &["explore_frontend", "explore_backend"],
                    ),
                ],
            ),
        )
        .await
        .expect("run should complete");

    assert_eq!(snapshot.run.state, RunState::Succeeded);
    assert_eq!(state_of(&snapshot, "contract"), NodeState::Succeeded);
}

#[tokio::test]
async fn the_concurrency_governor_really_limits_parallel_agents() {
    let executor = FakeExecutor::new().with_delay(Duration::from_millis(20));
    let harness = harness(
        executor.clone(),
        all_roles_claude(),
        EngineConfig {
            limits: ConcurrencyLimits {
                global: 2,
                per_project: 4,
                per_provider: 4,
            },
            slot_wait_millis: 5,
            ..EngineConfig::default()
        },
    );
    let snapshot = harness
        .engine
        .start_and_run(
            ProjectId::new(),
            &template(
                "wide",
                vec![
                    node("a", EXPLORER, &[]),
                    node("b", EXPLORER, &[]),
                    node("c", EXPLORER, &[]),
                    node("d", EXPLORER, &[]),
                    node("merge", IMPLEMENTER, &["a", "b", "c", "d"]),
                ],
            ),
        )
        .await
        .expect("run should complete");

    assert_eq!(snapshot.run.state, RunState::Succeeded);
    assert_eq!(
        executor.peak_concurrency(),
        2,
        "a global cap of 2 must be exactly respected, not approached"
    );
    assert_eq!(
        snapshot.attempts.len(),
        5,
        "every node runs exactly once when nothing fails"
    );
}

// --- failure and retry --------------------------------------------------

#[tokio::test]
async fn a_failing_dependency_skips_everything_downstream() {
    let executor =
        FakeExecutor::new().script("explore", vec![Behaviour::permanent("cannot reproduce")]);
    let harness = harness_with(executor.clone(), all_roles_claude());
    let snapshot = harness
        .engine
        .start_and_run(
            ProjectId::new(),
            &template(
                "broken",
                vec![
                    node("explore", EXPLORER, &[]),
                    node("implement", IMPLEMENTER, &["explore"]),
                    node("verify", TESTER, &["implement"]),
                    node("unrelated", TESTER, &[]),
                ],
            ),
        )
        .await
        .expect("a failed node is not an engine error");

    assert_eq!(snapshot.run.state, RunState::Failed);
    assert_eq!(state_of(&snapshot, "explore"), NodeState::Failed);
    assert_eq!(state_of(&snapshot, "implement"), NodeState::Skipped);
    assert_eq!(state_of(&snapshot, "verify"), NodeState::Skipped);
    // Independent work still ran: a failure in one branch does not cancel
    // the rest of the graph.
    assert_eq!(state_of(&snapshot, "unrelated"), NodeState::Succeeded);
    assert!(!executor.executed("implement"));
    let note = snapshot.run.note.unwrap_or_default();
    assert!(note.contains("implement"), "{note}");
    assert!(note.contains("verify"), "{note}");
}

#[tokio::test]
async fn a_retryable_failure_is_retried_after_a_backoff_and_can_succeed() {
    let executor = FakeExecutor::new().script(
        "implement",
        vec![Behaviour::retryable("provider timed out"), Behaviour::ok()],
    );
    let harness = harness(executor, all_roles_claude(), fast_config());
    let started_at = harness.clock.now_millis();
    let snapshot = harness
        .engine
        .start_and_run(
            ProjectId::new(),
            &template("retry", vec![node("implement", IMPLEMENTER, &[])]),
        )
        .await
        .expect("run should complete");

    assert_eq!(snapshot.run.state, RunState::Succeeded);
    assert_eq!(
        snapshot.node("implement").unwrap().attempts,
        2,
        "one failed attempt and one successful one"
    );
    let attempts = &snapshot.attempts;
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].trigger, AttemptTrigger::Initial);
    assert_eq!(attempts[0].finished_state, Some(NodeState::Failed));
    assert_eq!(attempts[1].trigger, AttemptTrigger::Retry);
    assert_eq!(attempts[1].finished_state, Some(NodeState::Succeeded));
    // The backoff was real: the deterministic clock had to move by the
    // policy's base delay before the retry was allowed to start.
    let elapsed = harness.clock.now_millis() - started_at;
    assert!(
        elapsed >= 5_000,
        "the retry should have waited out the 5s base backoff, elapsed {elapsed}ms"
    );
}

#[tokio::test]
async fn a_node_declared_non_retryable_is_not_retried() {
    let mut node = node("integrate", INTEGRATOR, &[]);
    node.retryable = false;
    let executor =
        FakeExecutor::new().script("integrate", vec![Behaviour::retryable("push refused")]);
    let harness = harness(executor.clone(), all_roles_claude(), fast_config());
    let snapshot = harness
        .engine
        .start_and_run(ProjectId::new(), &template("no-retry", vec![node]))
        .await
        .expect("run should complete");

    assert_eq!(snapshot.run.state, RunState::Failed);
    assert_eq!(snapshot.attempts.len(), 1);
    let detail = snapshot
        .node("integrate")
        .unwrap()
        .last_detail
        .clone()
        .unwrap_or_default();
    assert!(detail.contains("non-retryable"), "{detail}");
}

#[tokio::test]
async fn a_failure_the_executor_calls_permanent_is_not_retried() {
    let executor = FakeExecutor::new().script(
        "implement",
        vec![Behaviour::permanent("invalid instruction")],
    );
    let harness = harness(executor, all_roles_claude(), fast_config());
    let snapshot = harness
        .engine
        .start_and_run(
            ProjectId::new(),
            &template("permanent", vec![node("implement", IMPLEMENTER, &[])]),
        )
        .await
        .expect("run should complete");

    assert_eq!(snapshot.run.state, RunState::Failed);
    assert_eq!(snapshot.attempts.len(), 1);
    assert_eq!(
        snapshot.attempts[0].detail.as_deref(),
        Some("invalid instruction")
    );
}

#[tokio::test]
async fn attempts_stop_at_the_policy_limit_and_every_try_is_recorded() {
    let executor = FakeExecutor::new().script(
        "implement",
        vec![
            Behaviour::retryable("timeout 1"),
            Behaviour::retryable("timeout 2"),
            Behaviour::retryable("timeout 3"),
        ],
    );
    let harness = harness(
        executor,
        all_roles_claude(),
        EngineConfig {
            retry: RetryPolicy {
                max_attempts: 2,
                base_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(1),
            },
            ..EngineConfig::default()
        },
    );
    let snapshot = harness
        .engine
        .start_and_run(
            ProjectId::new(),
            &template("exhaust", vec![node("implement", IMPLEMENTER, &[])]),
        )
        .await
        .expect("run should complete");

    assert_eq!(snapshot.run.state, RunState::Failed);
    assert_eq!(
        snapshot.attempts.len(),
        2,
        "max_attempts = 2 means two rows"
    );
    assert_eq!(
        snapshot
            .attempts
            .iter()
            .filter(|attempt| attempt.finished_state == Some(NodeState::Succeeded))
            .count(),
        0
    );
    assert_eq!(snapshot.node("implement").unwrap().attempts, 2);
}

#[tokio::test]
async fn a_provider_that_is_missing_entirely_is_retried_then_reported() {
    let executor = FakeExecutor::new().with_missing_provider();
    let harness = harness(
        executor,
        all_roles_claude(),
        EngineConfig {
            retry: RetryPolicy {
                max_attempts: 2,
                base_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(1),
            },
            ..EngineConfig::default()
        },
    );
    let snapshot = harness
        .engine
        .start_and_run(
            ProjectId::new(),
            &template("no-cli", vec![node("implement", IMPLEMENTER, &[])]),
        )
        .await
        .expect("run should complete");

    assert_eq!(snapshot.run.state, RunState::Failed);
    assert_eq!(snapshot.attempts.len(), 2);
    let detail = snapshot
        .node("implement")
        .unwrap()
        .last_detail
        .clone()
        .unwrap();
    assert!(detail.contains("not found on PATH"), "{detail}");
}

#[tokio::test]
async fn an_executor_that_panics_is_reported_rather_than_swallowed() {
    let executor = FakeExecutor::new().panicking_on("implement");
    let harness = harness_with(executor, all_roles_claude());
    let error = harness
        .engine
        .start_and_run(
            ProjectId::new(),
            &template("panic", vec![node("implement", IMPLEMENTER, &[])]),
        )
        .await
        .expect_err("a panicking node task must surface as an error");
    assert!(
        matches!(error, OrchestratorError::NodeTaskPanicked { .. }),
        "unexpected error: {error}"
    );
}

// --- approval gates -----------------------------------------------------

#[tokio::test]
async fn an_approval_gate_stops_the_run_until_a_human_answers() {
    let executor = FakeExecutor::new();
    let harness = harness_with(executor.clone(), all_roles_claude());
    let snapshot = harness
        .engine
        .start_and_run(
            ProjectId::new(),
            &template(
                "gated",
                vec![
                    node("implement", IMPLEMENTER, &[]),
                    gated(node("integrate", INTEGRATOR, &["implement"])),
                ],
            ),
        )
        .await
        .expect("run should stop at the gate");

    assert_eq!(snapshot.run.state, RunState::AwaitingApproval);
    assert_eq!(state_of(&snapshot, "integrate"), NodeState::Pending);
    assert_eq!(snapshot.pending_approvals().len(), 1);
    assert!(
        !executor.executed("integrate"),
        "a gated node must not run before it is approved"
    );
    let approval = snapshot.pending_approvals()[0];
    assert!(
        approval.summary.contains("integrate"),
        "{}",
        approval.summary
    );

    // Running the engine again must not sneak past the gate.
    let again = harness.engine.run(snapshot.run.id).await.unwrap();
    assert_eq!(again.run.state, RunState::AwaitingApproval);
    assert!(!executor.executed("integrate"));

    let finished = harness
        .engine
        .approve_and_resume(snapshot.run.id, approval.id, "faisal")
        .await
        .expect("approving should resume the run");
    assert_eq!(finished.run.state, RunState::Succeeded);
    assert_eq!(state_of(&finished, "integrate"), NodeState::Succeeded);
    assert_eq!(executor.requests("integrate").len(), 1);
    assert!(
        finished
            .checkpoints
            .iter()
            .any(|checkpoint| checkpoint.detail.contains("approved by faisal")),
        "the approval decision must be in the checkpoint history"
    );
}

#[tokio::test]
async fn a_rejected_approval_cancels_the_run_and_never_runs_the_node() {
    let executor = FakeExecutor::new();
    let harness = harness_with(executor.clone(), all_roles_claude());
    let snapshot = harness
        .engine
        .start_and_run(
            ProjectId::new(),
            &template("gated", vec![gated(node("integrate", INTEGRATOR, &[]))]),
        )
        .await
        .expect("run should stop at the gate");

    let approval = snapshot.pending_approvals()[0].clone();
    harness
        .engine
        .decide_approval(
            snapshot.run.id,
            approval.id,
            &nacc_domain::ApprovalDecision::Rejected {
                by: "faisal".to_string(),
                reason: "not this release".to_string(),
            },
        )
        .await
        .expect("the decision is recordable");

    let after = harness.engine.snapshot(snapshot.run.id).await.unwrap();
    assert_eq!(after.run.state, RunState::Cancelled);
    assert_eq!(state_of(&after, "integrate"), NodeState::Cancelled);
    assert!(after.run.note.unwrap().contains("not this release"));
    assert!(!executor.executed("integrate"));

    // A rejected gate is final: resuming must not resurrect it.
    let resumed = harness.engine.resume(snapshot.run.id).await.unwrap();
    assert_eq!(resumed.run.state, RunState::Cancelled);
    assert!(!executor.executed("integrate"));
}

#[tokio::test]
async fn an_approval_cannot_be_decided_twice() {
    let harness = harness_with(FakeExecutor::new(), all_roles_claude());
    let snapshot = harness
        .engine
        .start_and_run(
            ProjectId::new(),
            &template("gated", vec![gated(node("integrate", INTEGRATOR, &[]))]),
        )
        .await
        .unwrap();
    let approval = snapshot.pending_approvals()[0].clone();
    let approve = nacc_domain::ApprovalDecision::Approved {
        by: "faisal".to_string(),
    };
    harness
        .engine
        .decide_approval(snapshot.run.id, approval.id, &approve)
        .await
        .expect("the first decision is recorded");
    let second = harness
        .engine
        .decide_approval(snapshot.run.id, approval.id, &approve)
        .await;
    assert!(second.is_err(), "an approval decision must be immutable");
}

// --- routing, permissions, and providers --------------------------------

#[tokio::test]
async fn an_unassigned_role_uses_its_declared_fallback_and_records_why() {
    let mut node = node("implement", IMPLEMENTER, &[]);
    node.fallbacks = vec![NodeFallback {
        provider_id: ProviderId::Codex,
        model_id: None,
        reason: "Claude is not configured on this machine".to_string(),
    }];
    let harness = harness_with(FakeExecutor::new(), StaticRouting::new());
    let snapshot = harness
        .engine
        .start_and_run(ProjectId::new(), &template("fallback", vec![node]))
        .await
        .expect("the fallback should carry the run");

    assert_eq!(snapshot.run.state, RunState::Succeeded);
    assert_eq!(snapshot.attempts[0].trigger, AttemptTrigger::Fallback);
    assert_eq!(snapshot.attempts[0].provider, Some(ProviderId::Codex));
}

#[tokio::test]
async fn an_unassigned_role_without_a_fallback_fails_with_the_reason() {
    let harness = harness_with(FakeExecutor::new(), StaticRouting::new());
    let snapshot = harness
        .engine
        .start_and_run(
            ProjectId::new(),
            &template("unassigned", vec![node("implement", IMPLEMENTER, &[])]),
        )
        .await
        .expect("an unconfigured role is a run failure, not an engine error");

    assert_eq!(snapshot.run.state, RunState::Failed);
    assert_eq!(
        snapshot.attempts.len(),
        1,
        "retrying an unconfigured role cannot help, so it is not retried"
    );
    let detail = snapshot
        .node("implement")
        .unwrap()
        .last_detail
        .clone()
        .unwrap();
    assert!(detail.contains("no provider assigned"), "{detail}");
}

#[tokio::test]
async fn a_role_profile_may_narrow_a_nodes_permission_but_never_widen_it() {
    let executor = FakeExecutor::new();
    let routing = StaticRouting::new()
        .assign_all(
            &[EXPLORER, IMPLEMENTER, TESTER, INTEGRATOR],
            ProviderId::Claude,
        )
        .with(
            IMPLEMENTER.clone(),
            RoleAssignment {
                provider_id: Some(ProviderId::Claude),
                model_id: None,
                permission_profile: Some(PermissionProfile::ReadOnly),
            },
        )
        .with(
            EXPLORER.clone(),
            RoleAssignment {
                provider_id: Some(ProviderId::Claude),
                model_id: None,
                permission_profile: Some(PermissionProfile::TemporaryDangerFullAccess),
            },
        );
    let harness = harness_with(executor.clone(), routing);
    harness
        .engine
        .start_and_run(
            ProjectId::new(),
            &template(
                "permissions",
                vec![
                    node("implement", IMPLEMENTER, &[]),
                    // Declares ReadOnly, so an expansive role row must not be
                    // able to raise it.
                    {
                        let mut n = node("explore", EXPLORER, &["implement"]);
                        n.permission_profile_hint = PermissionProfile::ReadOnly;
                        n
                    },
                ],
            ),
        )
        .await
        .expect("run should complete");

    let implement = executor.requests("implement");
    assert_eq!(
        implement[0].permission_profile,
        PermissionProfile::ReadOnly,
        "the role's narrower profile must win"
    );
    let explore = executor.requests("explore");
    assert_eq!(
        explore[0].permission_profile,
        PermissionProfile::ReadOnly,
        "the node's own declaration must win over a wider role profile"
    );
}

// --- pause, cancel, resume ----------------------------------------------

#[tokio::test]
async fn a_paused_run_does_nothing_until_it_is_resumed() {
    let executor = FakeExecutor::new();
    let harness = harness_with(executor.clone(), all_roles_claude());
    let snapshot = harness
        .engine
        .start_run(
            ProjectId::new(),
            &template("pausable", vec![node("implement", IMPLEMENTER, &[])]),
        )
        .await
        .unwrap();
    let paused = harness
        .engine
        .pause(snapshot.run.id, "user asked to stop")
        .await
        .unwrap();
    assert_eq!(paused.run.state, RunState::Paused);

    // `run` on a paused run reports the state instead of quietly starting it.
    let unchanged = harness.engine.run(snapshot.run.id).await.unwrap();
    assert_eq!(unchanged.run.state, RunState::Paused);
    assert!(!executor.executed("implement"));

    let open = harness.engine.open_runs().await.unwrap();
    assert!(open.iter().any(|run| run.id == snapshot.run.id));
    let attention = harness.engine.runs_needing_attention().await.unwrap();
    assert!(!attention.iter().any(|run| run.id == snapshot.run.id));

    let resumed = harness.engine.resume(snapshot.run.id).await.unwrap();
    assert_eq!(resumed.run.state, RunState::Succeeded);
    assert!(executor.executed("implement"));
}

#[tokio::test]
async fn cancelling_ends_the_run_and_marks_what_never_ran() {
    let executor = FakeExecutor::new();
    let harness = harness_with(executor.clone(), all_roles_claude());
    let snapshot = harness
        .engine
        .start_run(
            ProjectId::new(),
            &template(
                "cancellable",
                vec![
                    node("implement", IMPLEMENTER, &[]),
                    node("verify", TESTER, &["implement"]),
                ],
            ),
        )
        .await
        .unwrap();
    let cancelled = harness
        .engine
        .cancel(snapshot.run.id, "wrong project")
        .await
        .unwrap();
    assert_eq!(cancelled.run.state, RunState::Cancelled);
    for key in ["implement", "verify"] {
        assert_eq!(state_of(&cancelled, key), NodeState::Cancelled, "{key}");
    }
    assert!(cancelled.run.note.unwrap().contains("wrong project"));

    // Terminal means terminal: resume reports the state rather than reviving
    // the run.
    let resumed = harness.engine.resume(snapshot.run.id).await.unwrap();
    assert_eq!(resumed.run.state, RunState::Cancelled);
    assert!(!executor.executed("implement"));
}

#[tokio::test]
async fn resuming_a_run_that_is_already_running_is_refused() {
    let harness = harness_with(FakeExecutor::new(), all_roles_claude());
    let snapshot = harness
        .engine
        .start_run(
            ProjectId::new(),
            &template("running", vec![node("implement", IMPLEMENTER, &[])]),
        )
        .await
        .unwrap();
    let mut run = snapshot.run.clone();
    run.state = RunState::Running;
    harness.db.update_workflow_run(&run).await.unwrap();

    let error = harness
        .engine
        .resume(snapshot.run.id)
        .await
        .expect_err("a running run is owned by whichever engine is driving it");
    assert!(matches!(error, OrchestratorError::NotResumable { .. }));
}

// --- validation ---------------------------------------------------------

#[tokio::test]
async fn a_template_that_cannot_be_scheduled_is_refused_before_anything_is_written() {
    let harness = harness_with(FakeExecutor::new(), all_roles_claude());
    let error = harness
        .engine
        .start_run(
            ProjectId::new(),
            &template(
                "cyclic",
                vec![
                    node("a", IMPLEMENTER, &["b"]),
                    node("b", IMPLEMENTER, &["a"]),
                ],
            ),
        )
        .await
        .expect_err("a cyclic template must be refused");
    assert!(matches!(error, OrchestratorError::InvalidGraph { .. }));
    assert!(harness.engine.open_runs().await.unwrap().is_empty());
}

// --- recovery -----------------------------------------------------------

#[tokio::test]
async fn recovery_turns_a_crashed_run_into_a_resumable_interrupted_run() {
    let executor = FakeExecutor::new();
    let harness = harness_with(executor.clone(), all_roles_claude());
    let snapshot = harness
        .engine
        .start_run(
            ProjectId::new(),
            &template(
                "crashy",
                vec![
                    node("implement", IMPLEMENTER, &[]),
                    node("verify", TESTER, &["implement"]),
                ],
            ),
        )
        .await
        .unwrap();

    // Simulate the exact fingerprint of a crash: the run is `Running`, one
    // node is `Running`, and its attempt row was never finished.
    let mut run = snapshot.run.clone();
    run.state = RunState::Running;
    harness.db.update_workflow_run(&run).await.unwrap();
    let node_run_id = snapshot.node("implement").unwrap().id;
    let mut node_run = snapshot.node("implement").unwrap().clone();
    node_run.state = NodeState::Running;
    node_run.attempts = 1;
    harness.db.update_node_run(&node_run).await.unwrap();
    harness
        .db
        .insert_node_attempt(&NodeAttemptRecord {
            id: AttemptId::new(),
            node_run_id,
            workflow_run_id: run.id,
            attempt_number: 1,
            trigger: AttemptTrigger::Initial,
            finished_state: None,
            provider: Some(ProviderId::Claude),
            detail: None,
            started_at_millis: 1_000,
            finished_at_millis: None,
        })
        .await
        .unwrap();

    let recovered = recovery::reconcile(&harness.engine).await.unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].state_before, RunState::Running);
    assert_eq!(recovered[0].requeued_nodes, 1);
    assert_eq!(recovered[0].interrupted_attempts, 1);

    let after = harness.engine.snapshot(run.id).await.unwrap();
    assert_eq!(after.run.state, RunState::Interrupted);
    assert_eq!(state_of(&after, "implement"), NodeState::Pending);
    assert_eq!(
        after.attempts[0].finished_state,
        Some(NodeState::Failed),
        "the orphaned attempt is closed rather than left dangling"
    );
    assert!(after.unfinished_attempts().is_empty());

    // Recovery is idempotent, and it never resumed anything on its own.
    assert!(recovery::reconcile(&harness.engine)
        .await
        .unwrap()
        .is_empty());
    assert!(!executor.executed("implement"));

    // Resuming after recovery finishes the work, and the resume is recorded
    // as a recovery trigger rather than a plain retry. Three attempts make
    // up the run's history: the orphaned one recovery closed, the recovery
    // attempt for `implement`, and `verify`'s initial attempt once its
    // dependency finally succeeded.
    let resumed = harness.engine.resume(run.id).await.unwrap();
    assert_eq!(resumed.run.state, RunState::Succeeded);
    assert_eq!(resumed.attempts.len(), 3);
    assert_eq!(resumed.attempts[1].trigger, AttemptTrigger::Recovery);
    assert_eq!(
        resumed.attempts[1].finished_state,
        Some(NodeState::Succeeded)
    );
}

#[tokio::test]
async fn recovery_leaves_a_run_waiting_on_an_approval_resumable() {
    let harness = harness_with(FakeExecutor::new(), all_roles_claude());
    let snapshot = harness
        .engine
        .start_and_run(
            ProjectId::new(),
            &template("gated", vec![gated(node("integrate", INTEGRATOR, &[]))]),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.run.state, RunState::AwaitingApproval);

    let recovered = recovery::reconcile(&harness.engine).await.unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].state_before, RunState::AwaitingApproval);
    let after = harness.engine.snapshot(snapshot.run.id).await.unwrap();
    assert_eq!(after.run.state, RunState::Interrupted);
    assert_eq!(
        after.pending_approvals().len(),
        1,
        "the open request survives the restart"
    );

    // The gate is still a gate: resuming without a decision must not run it.
    let still_waiting = harness.engine.resume(snapshot.run.id).await.unwrap();
    assert_eq!(still_waiting.run.state, RunState::AwaitingApproval);
    assert!(still_waiting.pending_approvals().len() == 1);
}

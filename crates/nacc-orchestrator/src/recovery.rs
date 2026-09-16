//! Crash recovery (master plan S16): turn "NACC exited while this was
//! running" into a state a human can act on.
//!
//! # What recovery refuses to do
//!
//! It never resumes work by itself. A run found mid-flight is moved to
//! `Interrupted` with a reason, its orphaned attempt rows are closed as
//! failed, and its `Running` nodes are re-queued -- then it stops. Deciding to
//! continue is a human decision, because "the app started again" is not
//! evidence that the agent's half-finished edit in a worktree is still valid
//! (that is what `nacc-worktree`'s quarantine and reconciliation are for).
//!
//! Recovery *is* idempotent: running it twice is the same as running it once,
//! so a crash during recovery costs nothing.

use nacc_domain::{RunState, WorkflowRunId};

use crate::engine::WorkflowEngine;
use crate::Result;

/// One run that a previous process left behind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveredRun {
    pub run_id: WorkflowRunId,
    pub state_before: RunState,
    pub requeued_nodes: usize,
    pub interrupted_attempts: usize,
}

/// Reconcile every run a dead process left behind. Call once, at startup,
/// before the UI lists runs.
pub async fn reconcile(engine: &WorkflowEngine) -> Result<Vec<RecoveredRun>> {
    let mut recovered = Vec::new();
    for run in engine
        .runs_needing_attention()
        .await?
        .into_iter()
        // `AwaitingApproval` is included: the gate is still open, which is a
        // resumable state, but the process that asked for the approval is
        // gone and the run must not look like it is still moving.
        .filter(|run| matches!(run.state, RunState::Running | RunState::AwaitingApproval))
    {
        let state_before = run.state;
        let snapshot = engine.snapshot(run.id).await?;
        let interrupted_attempts = snapshot.unfinished_attempts().len();
        let requeued_nodes = engine.requeue_unfinished(run.id).await?;
        let note = match state_before {
            RunState::AwaitingApproval => {
                "NACC exited while this run was waiting for approval; the request is still open"
            }
            _ => "NACC exited while this run was active",
        };
        engine
            .mark_interrupted(run.id, note, requeued_nodes, interrupted_attempts)
            .await?;
        recovered.push(RecoveredRun {
            run_id: run.id,
            state_before,
            requeued_nodes,
            interrupted_attempts,
        });
    }
    if !recovered.is_empty() {
        tracing::warn!(
            count = recovered.len(),
            "recovered workflow runs that a previous process left in flight"
        );
    }
    Ok(recovered)
}

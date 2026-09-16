//! The durable DAG workflow engine: state machine, checkpointing, concurrency
//! governor, and crash recovery (master plan S14, S16; build prompt S17's
//! Phase 7).
//!
//! The pieces, and why they are separate:
//!
//! - [`scheduler`] is pure: given node definitions and node states, it says
//!   what may run, what can never run, and how the run ended. No database, no
//!   clock, no agent -- so the part of the engine most likely to be subtly
//!   wrong is the part that is cheapest to test exhaustively.
//! - [`governor`] is pure accounting: how many agents may run at once,
//!   globally, per project, and per provider.
//! - [`clock`] is the only source of time, so retry backoff is testable
//!   without waiting and every persisted timestamp has one definition.
//! - [`engine`] is the state machine: it reads durable state, decides the next
//!   legal transition, records it, and only then acts.
//! - [`recovery`] is what runs at startup: it explains what a dead process
//!   left behind and makes it resumable, but never resumes anything by itself.
//! - [`template`] holds the built-in presets (master plan S18) as real DAGs,
//!   because a preset that is only prose cannot be executed or checked.
//!
//! Two boundaries are injected rather than implemented here, on purpose:
//! [`engine::NodeExecutor`] (how an agent is actually launched) and
//! [`engine::RoleRouting`] (which provider, model, and permission a role
//! gets). That keeps this crate free of provider-specific knowledge and lets
//! its tests run the whole state machine without a single agent installed.

pub mod clock;
pub mod engine;
pub mod governor;
pub mod recovery;
pub mod scheduler;
pub mod template;

pub use clock::{Clock, SystemClock, VirtualClock};
pub use engine::{
    EngineConfig, NodeExecutionFailure, NodeExecutionOutcome, NodeExecutionRequest, NodeExecutor,
    RetryPolicy, RoleAssignment, RoleRouting, RunSnapshot, StaticRouting, WorkflowEngine,
};
pub use governor::{Capacity, ConcurrencyLimits, Governor, Permit, SlotKey};
pub use scheduler::{NodeStates, Outcome, Readiness};
pub use template::{built_in_template, built_in_templates, cross_provider_fallback};

use nacc_domain::{RunState, WorkflowRunId};
use nacc_storage::StorageError;

/// The engine's error vocabulary. Concrete variants rather than a single
/// string, because the GUI's job is to say something useful: "this role has
/// no provider assigned" and "this run's graph is broken" need different
/// answers from the user.
#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("storage failure: {0}")]
    Storage(#[from] StorageError),

    /// A template that could not be scheduled: a cycle, a dependency on
    /// something that does not exist, duplicate keys, or no entry point.
    #[error("invalid workflow graph: {detail}")]
    InvalidGraph { detail: String },

    #[error("no such workflow run: {run_id}")]
    UnknownRun { run_id: WorkflowRunId },

    #[error("run {run_id} has no node `{node_key}`")]
    NodeNotInRun {
        run_id: WorkflowRunId,
        node_key: String,
    },

    #[error("run {run_id} is in state {state:?} and cannot be resumed")]
    NotResumable {
        run_id: WorkflowRunId,
        state: RunState,
    },

    /// A node task panicked. Reported rather than swallowed: a panic in an
    /// executor is a bug in that executor, and hiding it would turn it into a
    /// mysterious node failure.
    #[error("a node task for run {run_id} panicked: {detail}")]
    NodeTaskPanicked {
        run_id: WorkflowRunId,
        detail: String,
    },

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, OrchestratorError>;

#[cfg(test)]
mod tests {
    use super::*;
    use nacc_domain::WorkflowRunId;

    #[test]
    fn errors_are_specific_enough_for_the_gui_to_act_on() {
        let run_id = WorkflowRunId::new();
        let resumable = OrchestratorError::NotResumable {
            run_id,
            state: RunState::Succeeded,
        };
        assert!(resumable.to_string().contains("cannot be resumed"));

        let graph = OrchestratorError::InvalidGraph {
            detail: "dependency cycle involving a, b".to_string(),
        };
        assert!(graph.to_string().contains("dependency cycle"));

        let missing = OrchestratorError::NodeNotInRun {
            run_id,
            node_key: "implement".to_string(),
        };
        assert!(missing.to_string().contains("implement"));
    }

    #[test]
    fn every_built_in_template_survives_the_engines_own_validation() {
        // The templates and the engine must not be able to disagree about
        // what a valid graph is.
        for template in built_in_templates() {
            scheduler::validate(&template.nodes)
                .unwrap_or_else(|err| panic!("{}: {err}", template.name));
        }
    }
}

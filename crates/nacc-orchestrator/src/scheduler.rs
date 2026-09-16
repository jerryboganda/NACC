//! Pure DAG logic: which nodes are runnable, which can never run, and what
//! the run's overall outcome is (master plan S14.2/S14.4).
//!
//! Everything in this module is a pure function over node definitions and
//! node states. That is deliberate: this is the part of the engine that is
//! easiest to get subtly wrong (a dependency mistake silently serializes
//! work that should be parallel, or worse, runs a node whose input never
//! materialized), so it is the part that is testable without a database, a
//! clock, or an agent.
//!
//! Nodes are identified by `WorkflowNode::key`, and the engine is handed the
//! persisted node definitions rather than the template: a resumed run must
//! follow the graph it started with, even if the built-in template has since
//! changed (see `nacc_storage::NodeRunRecord::definition`).

use nacc_domain::{NodeState, WorkflowNode};

use crate::{OrchestratorError, Result};

/// Validate a graph before anything is persisted: unique keys, no dependency
/// on a node that does not exist, and no cycle.
pub fn validate(nodes: &[WorkflowNode]) -> Result<()> {
    if nodes.is_empty() {
        return Err(OrchestratorError::InvalidGraph {
            detail: "a workflow with no nodes can never produce a result".to_string(),
        });
    }
    for (index, node) in nodes.iter().enumerate() {
        if node.key.trim().is_empty() {
            return Err(OrchestratorError::InvalidGraph {
                detail: format!("node at index {index} has an empty key"),
            });
        }
        if nodes
            .iter()
            .skip(index + 1)
            .any(|other| other.key == node.key)
        {
            return Err(OrchestratorError::InvalidGraph {
                detail: format!("duplicate node key `{}`", node.key),
            });
        }
        if node.depends_on.iter().any(|dep| dep == &node.key) {
            return Err(OrchestratorError::InvalidGraph {
                detail: format!("node `{}` depends on itself", node.key),
            });
        }
        for dependency in &node.depends_on {
            if !nodes.iter().any(|other| &other.key == dependency) {
                return Err(OrchestratorError::InvalidGraph {
                    detail: format!("node `{}` depends on unknown node `{dependency}`", node.key),
                });
            }
        }
    }
    if let Some(cycle) = cycle_members(nodes) {
        return Err(OrchestratorError::InvalidGraph {
            detail: format!("dependency cycle involving {}", cycle.join(", ")),
        });
    }
    if !nodes.iter().any(|node| node.depends_on.is_empty()) {
        // Without an entry point the run could never start, which would
        // otherwise show up much later as a workflow that hangs.
        return Err(OrchestratorError::InvalidGraph {
            detail: "no node is free of dependencies, so the workflow has no entry point"
                .to_string(),
        });
    }
    Ok(())
}

/// The nodes left over after repeatedly removing everything whose
/// dependencies are satisfied -- i.e. exactly the members of cycles, if any.
fn cycle_members(nodes: &[WorkflowNode]) -> Option<Vec<String>> {
    let mut resolved: Vec<&str> = Vec::new();
    let mut remaining: Vec<&WorkflowNode> = nodes.iter().collect();
    loop {
        let ready: Vec<&WorkflowNode> = remaining
            .iter()
            .copied()
            .filter(|node| {
                node.depends_on
                    .iter()
                    .all(|dep| resolved.iter().any(|key| key == dep))
            })
            .collect();
        if ready.is_empty() {
            break;
        }
        for node in &ready {
            resolved.push(node.key.as_str());
        }
        remaining.retain(|node| !ready.iter().any(|r| r.key == node.key));
    }
    if remaining.is_empty() {
        None
    } else {
        let mut keys: Vec<String> = remaining.iter().map(|node| node.key.clone()).collect();
        keys.sort();
        Some(keys)
    }
}

/// Whether a node may run, must wait, or can never run.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Readiness {
    /// Every dependency succeeded.
    Ready,
    /// At least one dependency is still pending or running.
    Waiting,
    /// A dependency ended in a state that cannot lead to success.
    Blocked,
}

pub fn readiness(node: &WorkflowNode, states: &NodeStates) -> Readiness {
    let mut waiting = false;
    for dependency in &node.depends_on {
        match states.get(dependency.as_str()) {
            // A dependency the engine has no row for cannot be assumed
            // satisfied: treating it as ready would run a node without its
            // input. Corrupt/partial state is surfaced as blocked instead.
            None => return Readiness::Blocked,
            Some(NodeState::Succeeded) => {}
            Some(NodeState::Pending) | Some(NodeState::Running) => waiting = true,
            Some(_) => return Readiness::Blocked,
        }
    }
    if waiting {
        Readiness::Waiting
    } else {
        Readiness::Ready
    }
}

/// Node states keyed by node key, the shape every function here consumes.
pub type NodeStates = std::collections::HashMap<String, NodeState>;

/// Nodes that can run right now, in the template's declaration order. The
/// order is stable so that an unconstrained graph dispatches deterministically
/// (tests, and a user comparing two runs, both depend on that).
pub fn runnable<'a>(nodes: &'a [WorkflowNode], states: &NodeStates) -> Vec<&'a WorkflowNode> {
    nodes
        .iter()
        .filter(|node| {
            states.get(&node.key) == Some(&NodeState::Pending)
                && readiness(node, states) == Readiness::Ready
        })
        .collect()
}

/// Pending nodes whose dependencies can never succeed, so the engine can mark
/// them `Skipped` instead of leaving them pending forever. Transitive by
/// construction: a node skipped here becomes a blocking dependency for the
/// next call, so repeated application reaches the whole downstream cone.
pub fn newly_blocked<'a>(nodes: &'a [WorkflowNode], states: &NodeStates) -> Vec<&'a WorkflowNode> {
    nodes
        .iter()
        .filter(|node| {
            states.get(&node.key) == Some(&NodeState::Pending)
                && readiness(node, states) == Readiness::Blocked
        })
        .collect()
}

/// Overall run outcome once no node can make further progress.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Outcome {
    /// Some node is still going to run (or is running).
    Running,
    Succeeded,
    Failed,
}

pub fn outcome(nodes: &[WorkflowNode], states: &NodeStates) -> Outcome {
    if nodes.iter().any(|node| {
        !states
            .get(&node.key)
            .copied()
            .unwrap_or(NodeState::Pending)
            .is_terminal()
    }) {
        return Outcome::Running;
    }
    if nodes
        .iter()
        .all(|node| states.get(&node.key) == Some(&NodeState::Succeeded))
    {
        Outcome::Succeeded
    } else {
        Outcome::Failed
    }
}

/// Node keys that ended unsuccessfully, for a run-failure note a human can
/// read without opening the database.
pub fn failures(nodes: &[WorkflowNode], states: &NodeStates) -> Vec<String> {
    nodes
        .iter()
        .filter(|node| {
            matches!(
                states.get(&node.key),
                Some(NodeState::Failed) | Some(NodeState::Skipped) | Some(NodeState::Cancelled)
            )
        })
        .map(|node| node.key.clone())
        .collect()
}

/// True when every node key in `nodes` has a state -- the cheap sanity check
/// the engine performs after creating a run.
pub fn fully_instantiated(nodes: &[WorkflowNode], states: &NodeStates) -> bool {
    nodes.iter().all(|node| states.contains_key(&node.key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nacc_domain::{PermissionProfile, RoleKind};

    fn node(key: &str, depends_on: &[&str]) -> WorkflowNode {
        WorkflowNode {
            key: key.to_string(),
            title: format!("node {key}"),
            role: RoleKind::BackendImplementer,
            depends_on: depends_on.iter().map(|key| key.to_string()).collect(),
            instruction: format!("do the thing for {key}"),
            permission_profile_hint: PermissionProfile::AutonomousWorktree,
            retryable: true,
            requires_approval: false,
            fallbacks: vec![],
        }
    }

    fn states(pairs: &[(&str, NodeState)]) -> NodeStates {
        pairs
            .iter()
            .map(|(key, state)| (key.to_string(), *state))
            .collect()
    }

    #[test]
    fn validation_rejects_every_shape_of_broken_graph() {
        assert!(validate(&[]).is_err(), "an empty graph must be rejected");
        assert!(
            validate(&[node("", &[])]).is_err(),
            "an empty key is not a key"
        );
        assert!(
            validate(&[node("a", &[]), node("a", &[])]).is_err(),
            "duplicate keys must be rejected"
        );
        assert!(
            validate(&[node("a", &["a"])]).is_err(),
            "a self-dependency must be rejected"
        );
        assert!(
            validate(&[node("a", &["ghost"])]).is_err(),
            "a dependency on a missing node must be rejected"
        );
        assert!(
            validate(&[node("a", &["b"]), node("b", &["a"])]).is_err(),
            "a cycle must be rejected"
        );
        assert!(
            validate(&[node("a", &["b"]), node("b", &["c"]), node("c", &["a"])]).is_err(),
            "a longer cycle must be rejected"
        );
        assert!(
            validate(&[node("a", &["b"]), node("b", &[])]).is_ok(),
            "a diamond is not a cycle"
        );
        assert!(
            validate(&[node("a", &["b", "b"]), node("b", &[])]).is_ok(),
            "a repeated dependency is redundant, not invalid"
        );
    }

    #[test]
    fn the_cycle_error_names_the_nodes_involved() {
        let err = validate(&[node("a", &["b"]), node("b", &["a"]), node("c", &[])]).unwrap_err();
        let message = err.to_string();
        assert!(message.contains('a') && message.contains('b'), "{message}");
        assert!(!message.contains("cycle involving c"), "{message}");
    }

    #[test]
    fn a_node_runs_only_when_every_dependency_succeeded() {
        let nodes = vec![node("a", &[]), node("b", &["a"]), node("c", &["a", "b"])];
        // Fresh run: only the entry point is runnable.
        let fresh = states(&[
            ("a", NodeState::Pending),
            ("b", NodeState::Pending),
            ("c", NodeState::Pending),
        ]);
        assert_eq!(
            runnable(&nodes, &fresh)
                .iter()
                .map(|n| n.key.as_str())
                .collect::<Vec<_>>(),
            vec!["a"]
        );
        assert_eq!(readiness(&nodes[1], &fresh), Readiness::Waiting);
        assert!(newly_blocked(&nodes, &fresh).is_empty());

        // Half done: `b` is now runnable, `c` is still waiting on it.
        let partial = states(&[
            ("a", NodeState::Succeeded),
            ("b", NodeState::Pending),
            ("c", NodeState::Pending),
        ]);
        assert_eq!(
            runnable(&nodes, &partial)
                .iter()
                .map(|n| n.key.as_str())
                .collect::<Vec<_>>(),
            vec!["b"]
        );

        // A failed dependency blocks its dependents instead of letting them
        // run on missing input.
        let failed = states(&[
            ("a", NodeState::Failed),
            ("b", NodeState::Pending),
            ("c", NodeState::Pending),
        ]);
        assert!(runnable(&nodes, &failed).is_empty());
        assert_eq!(readiness(&nodes[1], &failed), Readiness::Blocked);
        assert_eq!(readiness(&nodes[2], &failed), Readiness::Blocked);
    }

    #[test]
    fn blocking_propagates_downstream_once_the_engine_marks_a_skip() {
        // The engine marks blocked nodes `Skipped` one layer at a time; this
        // proves repeated application reaches a three-deep cone, and that a
        // node with both a skipped and a running dependency stays blocked
        // rather than running early.
        let nodes = vec![
            node("root", &[]),
            node("mid", &["root"]),
            node("leaf", &["mid"]),
            node("side", &["root", "mid"]),
        ];
        let mut current = states(&[
            ("root", NodeState::Failed),
            ("mid", NodeState::Pending),
            ("leaf", NodeState::Pending),
            ("side", NodeState::Pending),
        ]);
        let mut passes = 0;
        loop {
            let blocked: Vec<String> = newly_blocked(&nodes, &current)
                .iter()
                .map(|node| node.key.clone())
                .collect();
            if blocked.is_empty() {
                break;
            }
            passes += 1;
            for key in blocked {
                current.insert(key, NodeState::Skipped);
            }
            assert!(passes <= 4, "block propagation did not terminate");
        }
        assert_eq!(current["mid"], NodeState::Skipped);
        assert_eq!(current["leaf"], NodeState::Skipped);
        assert_eq!(current["side"], NodeState::Skipped);
        assert_eq!(outcome(&nodes, &current), Outcome::Failed);
        assert_eq!(
            failures(&nodes, &current),
            vec!["root", "mid", "leaf", "side"]
        );
    }

    #[test]
    fn a_missing_node_row_is_blocked_not_silently_ready() {
        // Partial state (a crash between two writes) must not let a node run
        // without its dependency's output.
        let nodes = [node("a", &[]), node("b", &["a"])];
        let only_b = states(&[("b", NodeState::Pending)]);
        assert_eq!(readiness(&nodes[1], &only_b), Readiness::Blocked);
    }

    #[test]
    fn outcome_is_running_until_every_node_is_terminal() {
        let nodes = [node("a", &[]), node("b", &["a"])];
        let mid = states(&[("a", NodeState::Succeeded), ("b", NodeState::Running)]);
        assert_eq!(outcome(&nodes, &mid), Outcome::Running);
        let done = states(&[("a", NodeState::Succeeded), ("b", NodeState::Succeeded)]);
        assert_eq!(outcome(&nodes, &done), Outcome::Succeeded);
        assert!(fully_instantiated(&nodes, &done));

        // One success and one skip is a failure, not a success: a run that
        // silently loses a node is exactly the outcome the engine must never
        // report as green.
        let skipped = states(&[("a", NodeState::Succeeded), ("b", NodeState::Skipped)]);
        assert_eq!(outcome(&nodes, &skipped), Outcome::Failed);
    }

    #[test]
    fn parallel_branches_are_all_runnable_at_once() {
        let nodes = vec![
            node("left", &[]),
            node("right", &[]),
            node("merge", &["left", "right"]),
        ];
        let fresh = states(&[
            ("left", NodeState::Pending),
            ("right", NodeState::Pending),
            ("merge", NodeState::Pending),
        ]);
        assert_eq!(runnable(&nodes, &fresh).len(), 2);
        assert!(runnable(&nodes, &fresh)
            .iter()
            .all(|node| node.key != "merge"));
    }
}

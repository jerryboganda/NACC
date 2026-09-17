//! Built-in workflow templates: master plan S18's presets, expressed as real
//! DAGs (S14.2) rather than prose.
//!
//! Each preset here is the executable form of the plan's own description.
//! The graphs matter: the plan's presets are not a linear list of steps, and
//! the whole point of several of them is that work that does not depend on
//! each other runs *in parallel* (S14.4's concurrency governor) and that
//! independent roles produce independent evidence (S2.6).
//!
//! Every template obeys three rules, and each is asserted in the tests:
//!
//! 1. dependencies name real nodes, and the graph is acyclic;
//! 2. no node both mutates a repository and runs without an approval gate
//!    when it is classified as an always-approval-gated operation
//!    (integration, CI repair, release of a candidate);
//! 3. instructions are concrete enough to hand to an agent as-is.
//!
//! Templates are *data* (`nacc_domain::WorkflowTemplate`), so a user-authored
//! template needs no code change to run -- these are the built-ins.

use nacc_domain::{
    NodeFallback, PermissionProfile, ProviderId, RoleKind, WorkflowNode, WorkflowTemplate,
};

/// A node that may write inside its own worktree.
fn node(
    key: &str,
    title: &str,
    role: RoleKind,
    depends_on: &[&str],
    instruction: &str,
) -> WorkflowNode {
    WorkflowNode {
        key: key.to_string(),
        title: title.to_string(),
        role,
        depends_on: depends_on.iter().map(|key| key.to_string()).collect(),
        instruction: instruction.to_string(),
        permission_profile_hint: PermissionProfile::AutonomousWorktree,
        retryable: true,
        requires_approval: false,
        timeout_secs: None,
        fallbacks: vec![],
    }
}

/// A node whose role is purely investigative: exploration, planning,
/// research, and review all run under `ReadOnly`, so an agent that decides to
/// "helpfully" fix something it was asked to inspect is blocked by the
/// permission profile rather than by a prompt instruction (master plan
/// S2.5/S12.1).
fn readonly_node(
    key: &str,
    title: &str,
    role: RoleKind,
    depends_on: &[&str],
    instruction: &str,
) -> WorkflowNode {
    WorkflowNode {
        permission_profile_hint: PermissionProfile::ReadOnly,
        ..node(key, title, role, depends_on, instruction)
    }
}

/// Fast Bug Fix (master plan S18.2): explore and reproduce in parallel, then
/// implement, then verify, then review with a *different* provider, then
/// integrate behind an approval gate.
pub fn fast_bug_fix() -> WorkflowTemplate {
    WorkflowTemplate {
        name: "fast_bug_fix".to_string(),
        description:
            "Reproduce a bug, fix it in an isolated worktree, verify deterministically, review \
             with an independent provider, then integrate."
                .to_string(),
        nodes: vec![
            readonly_node(
                "explore",
                "Explore and reproduce",
                RoleKind::RepositoryExplorer,
                &[],
                "Read the repository and reproduce the reported bug. Produce the exact failing \
                 command, its output, and the smallest set of files involved. Do not modify any \
                 file.",
            ),
            readonly_node(
                "plan",
                "Plan the fix",
                RoleKind::ArchitectPlanner,
                &["explore"],
                "From the reproduction above, produce a minimal fix contract: files to change, \
                 the invariant to preserve, and the test that will prove the fix. State the root \
                 cause, not the symptom.",
            ),
            node(
                "implement",
                "Implement in a worktree",
                RoleKind::BackendImplementer,
                &["plan"],
                "Implement exactly the contract from the plan in this isolated worktree. Add the \
                 regression test it names. Commit locally when the tree is coherent.",
            ),
            node(
                "verify",
                "Deterministic verification",
                RoleKind::TestEngineer,
                &["implement"],
                "Run the repository's own quality gates against this worktree and report exact \
                 commands and exact results. Do not summarize a failure as a pass.",
            ),
            readonly_node(
                "review",
                "Independent review",
                RoleKind::GeneralCodeReviewer,
                &["verify"],
                "Review the diff against the plan with fresh eyes. Report findings as \
                 file:line plus a concrete failure scenario. Use a different provider than the \
                 implementer used.",
            ),
            WorkflowNode {
                // Integration is an always-approval-gated operation
                // (master plan S12.2): it writes to the repository.
                requires_approval: true,
                permission_profile_hint: PermissionProfile::RepositoryMaintainer,
                ..node(
                    "integrate",
                    "Integrate",
                    RoleKind::Integrator,
                    &["review"],
                    "Integrate the reviewed change onto the update path, running the same \
                     verification commands as `verify`. Never force-push.",
                )
            },
        ],
    }
}

/// Enterprise Feature (master plan S18.1): parallel exploration across
/// independent areas (frontend, backend, external research), a contract
/// derived from all three, parallel implementation, cross-provider review,
/// and a gated integration.
pub fn enterprise_feature() -> WorkflowTemplate {
    WorkflowTemplate {
        name: "enterprise_feature".to_string(),
        description: "Multi-area feature: parallel read-only exploration, a single implementation \
             contract, parallel implementation in separate worktrees, database migration review, \
             cross-provider review, then a gated integration."
            .to_string(),
        nodes: vec![
            readonly_node(
                "explore_frontend",
                "Explore frontend",
                RoleKind::RepositoryExplorer,
                &[],
                "Map the frontend surfaces this feature touches: components, state, routing, and \
                 the tests that cover them. Read-only.",
            ),
            readonly_node(
                "explore_backend",
                "Explore backend",
                RoleKind::RepositoryExplorer,
                &[],
                "Map the backend surfaces this feature touches: commands, storage, and the tests \
                 that cover them. Read-only.",
            ),
            readonly_node(
                "research_external",
                "Research upstream constraints",
                RoleKind::ExternalResearcher,
                &[],
                "Find the external constraints this feature must respect (provider CLI \
                 contracts, platform APIs, protocol limits). Cite each source.",
            ),
            readonly_node(
                "contract",
                "Implementation contract",
                RoleKind::ArchitectPlanner,
                &["explore_frontend", "explore_backend", "research_external"],
                "Reconcile the three explorations into one implementation contract: interfaces, \
                 migration plan, test plan, and explicit non-goals. Resolve conflicts rather than \
                 averaging them.",
            ),
            node(
                "implement_backend",
                "Implement backend",
                RoleKind::BackendImplementer,
                &["contract"],
                "Implement the backend half of the contract in this worktree, with its tests.",
            ),
            node(
                "implement_frontend",
                "Implement frontend",
                RoleKind::FrontendImplementer,
                &["contract"],
                "Implement the frontend half of the contract in this worktree, with its tests.",
            ),
            readonly_node(
                "review_migration",
                "Review database migration",
                RoleKind::DatabaseMigrationImplementer,
                &["implement_backend"],
                "Attack the migration specifically: forward path, rollback behavior, existing data \
                 with the old schema, and concurrent access. Report findings, do not rewrite the \
                 migration unless asked.",
            ),
            readonly_node(
                "review_code",
                "Cross-provider code review",
                RoleKind::GeneralCodeReviewer,
                &["implement_backend", "implement_frontend"],
                "Review both halves together for integration mismatches the individual authors \
                 could not see. Use a different provider than the implementers used.",
            ),
            WorkflowNode {
                requires_approval: true,
                permission_profile_hint: PermissionProfile::RepositoryMaintainer,
                ..node(
                    "integrate",
                    "Integrate",
                    RoleKind::Integrator,
                    &["review_migration", "review_code"],
                    "Integrate serially: backend first, then frontend, running the full \
                     verification suite after each. Stop on the first failure.",
                )
            },
        ],
    }
}

/// CI/CD Repair (master plan S18.3): diagnose the failing run, plan the
/// repair, fix it, and gate the push that would re-trigger CI.
pub fn ci_cd_repair() -> WorkflowTemplate {
    WorkflowTemplate {
        name: "ci_cd_repair".to_string(),
        description:
            "Diagnose a failing CI run from its logs, plan a minimal repair, implement it, \
             verify locally, then gate the push that re-triggers CI."
                .to_string(),
        nodes: vec![
            readonly_node(
                "diagnose",
                "Diagnose the failing run",
                RoleKind::CiCdInvestigator,
                &[],
                "Read the failing run's logs and identify the exact failing step, its exact \
                 error, and whether it is a code defect, an environment difference, or a flake. \
                 Quote the log lines that justify the conclusion.",
            ),
            readonly_node(
                "plan",
                "Plan the minimal repair",
                RoleKind::ArchitectPlanner,
                &["diagnose"],
                "Propose the smallest change that makes the step pass without weakening the \
                 check. Say explicitly what you are NOT changing.",
            ),
            node(
                "implement",
                "Implement the repair",
                RoleKind::BackendImplementer,
                &["plan"],
                "Implement the repair in this worktree and reproduce the CI step locally.",
            ),
            node(
                "verify",
                "Reproduce the CI step locally",
                RoleKind::TestEngineer,
                &["implement"],
                "Run the CI step's exact command locally and report its exact result.",
            ),
            WorkflowNode {
                // Pushing re-triggers CI on a real repository: gated.
                requires_approval: true,
                permission_profile_hint: PermissionProfile::CiMaintainer,
                ..node(
                    "push",
                    "Push the repair",
                    RoleKind::CiCdInvestigator,
                    &["verify"],
                    "Push the verified repair to the branch the failing run belongs to. Never \
                     push to a protected branch without explicit approval.",
                )
            },
        ],
    }
}

/// Read-Only Audit (master plan S18.6): parallel readers, a merged report,
/// and no permission to change anything.
pub fn read_only_audit() -> WorkflowTemplate {
    WorkflowTemplate {
        name: "read_only_audit".to_string(),
        description:
            "Independent parallel audits of the same codebase, merged into one report. Nothing \
             is written to the repository."
                .to_string(),
        nodes: vec![
            readonly_node(
                "audit_security",
                "Security audit",
                RoleKind::SecurityReviewer,
                &[],
                "Audit for security defects and abuses: trust boundaries, injection surfaces, \
                 credential handling, and unprotected privileged operations. Report with evidence.",
            ),
            readonly_node(
                "audit_performance",
                "Performance audit",
                RoleKind::PerformanceReviewer,
                &[],
                "Audit for performance defects: unbounded work, repeated I/O, quadratic \
                 behavior on realistic inputs. Report with evidence.",
            ),
            readonly_node(
                "audit_ux",
                "Accessibility and UX audit",
                RoleKind::AccessibilityUxReviewer,
                &[],
                "Audit the user-facing surfaces for accessibility and UX defects, including \
                 keyboard reachability and state that is shown but not operable.",
            ),
            readonly_node(
                "report",
                "Merge the audit report",
                RoleKind::DocumentationWriter,
                &["audit_security", "audit_performance", "audit_ux"],
                "Merge the three audits into one report ordered by severity. Where they disagree, \
                 say so instead of picking a side. Flag anything that would require a write to \
                 verify.",
            ),
        ],
    }
}

/// Frontend Visual Hardening (master plan S18.4): explore the UI, implement,
/// check the responsive/browser matrix and accessibility, review
/// independently, then integrate behind an approval gate.
pub fn frontend_visual_hardening() -> WorkflowTemplate {
    WorkflowTemplate {
        name: "frontend_visual_hardening".to_string(),
        description:
            "Harden the user-facing surfaces: implement in an isolated worktree, check the \
             responsive/browser matrix and accessibility, review with fresh eyes, then integrate."
                .to_string(),
        nodes: vec![
            readonly_node(
                "explore_ui",
                "Explore the UI surfaces",
                RoleKind::RepositoryExplorer,
                &[],
                "Map the user-facing components and routes: what renders them, what state they \
                 depend on, and where their styles live. Name the surfaces the change will touch.",
            ),
            node(
                "implement",
                "Implement the change",
                RoleKind::FrontendImplementer,
                &["explore_ui"],
                "Implement the visual change in this isolated worktree. Keep the component API \
                 stable unless the plan says otherwise, and commit locally when coherent.",
            ),
            node(
                "visual_matrix",
                "Responsive/browser matrix checks",
                RoleKind::TestEngineer,
                &["implement"],
                "Run the project's visual checks (Playwright or the repository's own harness) \
                 across the declared breakpoints and browsers. Report exact commands and exact \
                 results, including screenshots' paths. Do not summarize a failure as a pass.",
            ),
            readonly_node(
                "accessibility_review",
                "Accessibility review",
                RoleKind::AccessibilityUxReviewer,
                &["implement"],
                "Review the changed surfaces for accessibility: keyboard reachability, focus \
                 order, labels and roles, contrast, and state that is shown but not operable. \
                 Report findings as component plus concrete failure scenario.",
            ),
            readonly_node(
                "review",
                "Independent UI review",
                RoleKind::GeneralCodeReviewer,
                &["visual_matrix", "accessibility_review"],
                "Review the diff with fresh eyes against the matrix and accessibility findings. \
                 Use a different provider than the implementer used.",
            ),
            WorkflowNode {
                // Integration writes to the repository: always gated (S12.2).
                requires_approval: true,
                permission_profile_hint: PermissionProfile::RepositoryMaintainer,
                ..node(
                    "integrate",
                    "Integrate the change",
                    RoleKind::Integrator,
                    &["review"],
                    "Merge the reviewed worktree branch serially. CI runs after integration; \
                     this step is the human gate in front of that.",
                )
            },
        ],
    }
}

/// Backend Security Change (master plan S18.5): threat-first exploration,
/// implementation, migration and security review, deterministic tests, then
/// integration behind an approval gate.
pub fn backend_security_change() -> WorkflowTemplate {
    WorkflowTemplate {
        name: "backend_security_change".to_string(),
        description:
            "Make a security-relevant backend change: threat notes first, implementation in an \
             isolated worktree, migration and security review, deterministic tests, then \
             integrate."
                .to_string(),
        nodes: vec![
            readonly_node(
                "explore_security",
                "Explore the trust boundaries",
                RoleKind::RepositoryExplorer,
                &[],
                "Map the surfaces the change touches: inputs and their trust level, credential \
                 handling, privileged operations, and the existing tests that cover them.",
            ),
            readonly_node(
                "threat_notes",
                "Write threat notes",
                RoleKind::ArchitectPlanner,
                &["explore_security"],
                "Name the threats the change must not introduce (injection, confused deputy, \
                 privilege escalation, secret exposure) and the invariant each mitigation \
                 preserves. The implementation is checked against these notes.",
            ),
            node(
                "implement",
                "Implement the change",
                RoleKind::BackendImplementer,
                &["threat_notes"],
                "Implement the change in this isolated worktree, satisfying every invariant in \
                 the threat notes. Add the tests that would fail if an invariant regressed.",
            ),
            readonly_node(
                "review_migration",
                "Database/migration review",
                RoleKind::DatabaseMigrationImplementer,
                &["implement"],
                "Review any schema or data migration for reversibility, locking, and data loss. \
                 If there is no migration, say so explicitly rather than reviewing nothing.",
            ),
            readonly_node(
                "review_security",
                "Security review",
                RoleKind::SecurityReviewer,
                &["implement"],
                "Review the diff against the threat notes. Every named threat needs either a \
                 mitigation or an explicit, justified non-issue. Report findings as file:line \
                 plus a concrete abuse scenario.",
            ),
            node(
                "tests",
                "Deterministic tests",
                RoleKind::TestEngineer,
                &["review_migration", "review_security"],
                "Run the unit, integration, and authorization tests against this worktree and \
                 report exact commands and exact results. Do not summarize a failure as a pass.",
            ),
            WorkflowNode {
                // Integration writes to the repository: always gated (S12.2);
                // CI runs after it, which is the next stage outside this DAG.
                requires_approval: true,
                permission_profile_hint: PermissionProfile::RepositoryMaintainer,
                ..node(
                    "integrate",
                    "Integrate the change",
                    RoleKind::Integrator,
                    &["tests"],
                    "Merge the reviewed worktree branch serially so CI can run on it. This step \
                     is the human gate in front of CI.",
                )
            },
        ],
    }
}

/// The presets NACC ships, in the order the UI lists them. All six master
/// plan S18 subsections are here: fast_bug_fix (S18.2), ci_cd_repair
/// (S18.3), enterprise_feature (S18.1), frontend_visual_hardening (S18.4),
/// backend_security_change (S18.5), and read_only_audit (S18.6).
pub fn built_in_templates() -> Vec<WorkflowTemplate> {
    vec![
        fast_bug_fix(),
        ci_cd_repair(),
        enterprise_feature(),
        frontend_visual_hardening(),
        backend_security_change(),
        read_only_audit(),
    ]
}

/// Look one up by its stable name (which is what a run persists, so a
/// template's *content* may evolve while old runs keep their own node rows).
pub fn built_in_template(name: &str) -> Option<WorkflowTemplate> {
    built_in_templates()
        .into_iter()
        .find(|template| template.name == name)
}

/// A commonly useful fallback: when the primary provider is unavailable,
/// prefer a different vendor. Declared explicitly per node rather than
/// applied globally, because switching providers silently is exactly what
/// master plan S14.5 forbids.
pub fn cross_provider_fallback(from: ProviderId, reason: &str) -> Vec<NodeFallback> {
    let alternative = match from {
        ProviderId::Claude => ProviderId::Codex,
        ProviderId::Codex => ProviderId::Claude,
        ProviderId::Antigravity => ProviderId::Claude,
        ProviderId::Copilot => ProviderId::Claude,
        ProviderId::Opencode => ProviderId::Claude,
    };
    vec![NodeFallback {
        provider_id: alternative,
        model_id: None,
        reason: reason.to_string(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_built_in_template_is_a_valid_dag() {
        for template in built_in_templates() {
            let keys: HashSet<&str> = template.nodes.iter().map(|n| n.key.as_str()).collect();
            assert_eq!(
                keys.len(),
                template.nodes.len(),
                "{} has duplicate node keys",
                template.name
            );
            for node in &template.nodes {
                for dependency in &node.depends_on {
                    assert!(
                        keys.contains(dependency.as_str()),
                        "{}: node {} depends on unknown node {dependency}",
                        template.name,
                        node.key
                    );
                    assert_ne!(
                        dependency, &node.key,
                        "{}: node {} depends on itself",
                        template.name, node.key
                    );
                }
            }
            // Acyclicity: repeatedly remove nodes whose dependencies are all
            // already removed. Anything left is part of a cycle.
            let mut resolved: HashSet<&str> = HashSet::new();
            let mut remaining: Vec<&WorkflowNode> = template.nodes.iter().collect();
            loop {
                let ready: Vec<&WorkflowNode> = remaining
                    .iter()
                    .copied()
                    .filter(|node| {
                        node.depends_on
                            .iter()
                            .all(|dep| resolved.contains(dep.as_str()))
                    })
                    .collect();
                if ready.is_empty() {
                    break;
                }
                for node in &ready {
                    resolved.insert(node.key.as_str());
                }
                remaining.retain(|node| !ready.iter().any(|r| r.key == node.key));
            }
            assert!(
                remaining.is_empty(),
                "{} contains a dependency cycle: {:?}",
                template.name,
                remaining.iter().map(|n| n.key.as_str()).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn every_template_has_at_least_one_node_with_no_dependency() {
        // Otherwise the DAG could never start, which is a template bug that
        // would otherwise only show up as a workflow that hangs.
        for template in built_in_templates() {
            assert!(
                template.nodes.iter().any(|node| node.depends_on.is_empty()),
                "{} has no entry point",
                template.name
            );
        }
    }

    #[test]
    fn instructions_are_concrete_enough_to_hand_to_an_agent() {
        for template in built_in_templates() {
            for node in &template.nodes {
                assert!(
                    node.instruction.len() > 40,
                    "{}: node {} has a placeholder instruction",
                    template.name,
                    node.key
                );
                assert!(
                    !node.instruction.to_lowercase().contains("todo"),
                    "{}: node {} still contains a TODO",
                    template.name,
                    node.key
                );
            }
        }
    }

    #[test]
    fn repository_writing_nodes_are_approval_gated() {
        // Master plan S12.2 lists integration, pushing, and release as
        // always-approval-gated. Those are exactly the nodes whose keys say
        // so; this test makes the rule enforceable rather than a convention.
        for template in built_in_templates() {
            for node in &template.nodes {
                let writes_repository = matches!(
                    node.key.as_str(),
                    "integrate" | "push" | "release" | "publish"
                );
                assert_eq!(
                    node.requires_approval, writes_repository,
                    "{}: node {} must require approval exactly when it writes to the repository",
                    template.name, node.key
                );
            }
        }
    }

    #[test]
    fn presets_with_parallel_exploration_really_are_parallel() {
        // If the plan's parallel explorers were serialized by a dependency
        // mistake, the preset would still "work" and quietly lose the
        // property it exists for.
        let feature = enterprise_feature();
        let explorers: Vec<&WorkflowNode> = feature
            .nodes
            .iter()
            .filter(|node| node.key.starts_with("explore_") || node.key == "research_external")
            .collect();
        assert_eq!(explorers.len(), 3);
        for explorer in explorers {
            assert!(
                explorer.depends_on.is_empty(),
                "{} must be able to start immediately",
                explorer.key
            );
        }
        let implementers: Vec<&WorkflowNode> = feature
            .nodes
            .iter()
            .filter(|node| node.key.starts_with("implement_"))
            .collect();
        assert_eq!(implementers.len(), 2);
        for implementer in implementers {
            assert_eq!(implementer.depends_on, vec!["contract".to_string()]);
        }
    }

    #[test]
    fn a_read_only_audit_contains_no_approval_gate_because_it_writes_nothing() {
        let audit = read_only_audit();
        assert!(audit.nodes.iter().all(|node| !node.requires_approval));
        assert!(audit
            .nodes
            .iter()
            .all(|node| node.permission_profile_hint == PermissionProfile::ReadOnly));
    }

    #[test]
    fn lookup_by_name_round_trips_for_every_built_in() {
        for template in built_in_templates() {
            let found = built_in_template(&template.name)
                .unwrap_or_else(|| panic!("{} must be findable by name", template.name));
            assert_eq!(found.nodes.len(), template.nodes.len());
        }
        assert!(built_in_template("no_such_template").is_none());
    }

    #[test]
    fn cross_provider_fallback_never_picks_the_provider_that_failed() {
        for provider in [
            ProviderId::Claude,
            ProviderId::Codex,
            ProviderId::Antigravity,
            ProviderId::Copilot,
            ProviderId::Opencode,
        ] {
            let fallbacks = cross_provider_fallback(provider, "primary unavailable");
            assert_eq!(fallbacks.len(), 1);
            assert_ne!(
                fallbacks[0].provider_id, provider,
                "a fallback to the same provider is not a fallback"
            );
            assert!(!fallbacks[0].reason.is_empty());
        }
    }
}

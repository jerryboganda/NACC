//! Core domain model for NACC.
//!
//! Strongly typed IDs and shared value objects live here. Every other NACC
//! crate may depend on this one; this crate depends on nothing else in the
//! workspace, per the dependency-direction rule in the master plan (S6):
//! "Provider crates depend on a common provider core... the orchestrator
//! depends on abstractions, not concrete CLI parsers."
//!
//! Phase 1 defined only the ID types needed to carry a real, meaningful
//! value across the first typed IPC round trip (see `src-tauri`'s
//! `get_app_diagnostics` command). Phase 2 ("durable domain/storage/events",
//! master plan S4.4) adds the subset of the full ~29-entity domain model
//! (S7) that `nacc-storage` and `nacc-events` genuinely persist and query
//! now: `NodeRunId` and `AttemptId` for the correlation IDs S22 requires on
//! every event/audit record, `EventId` for the normalized event stream
//! (S6/S8.2), `AuditEventId` for the audit trail (S22). The rest of S7's
//! entities (`WorkflowTemplate`, `TaskContract`, `AgentHandoff`,
//! `WorktreeLease`, `QualityGateResult`, ...) are added by whichever later
//! phase's crate first has real logic that consumes them (see each
//! placeholder crate's own doc comment for its target phase) -- adding an
//! ID type with no real caller yet is exactly the kind of speculative code
//! this workspace has deliberately avoided since Phase 1 (see nacc-storage
//! and nacc-events' own doc comments on scope discipline).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Errors constructing or parsing domain value objects.
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("invalid {type_name} id {input:?}: {source}")]
    InvalidId {
        type_name: &'static str,
        input: String,
        #[source]
        source: uuid::Error,
    },
}

/// Defines a UUIDv4-backed strongly typed ID newtype.
///
/// Generates: `Copy, Clone, Eq, PartialEq, Hash, Debug, Display,
/// Serialize, Deserialize, specta::Type`, plus `new()` (random),
/// `from_uuid(Uuid)`, `as_uuid(&self) -> Uuid`, and `FromStr`. Two distinct
/// ID newtypes are never comparable or interchangeable even though both
/// wrap a `Uuid` -- that is the entire point of the pattern (master plan
/// S7: "Use strongly typed IDs rather than raw strings across the
/// backend").
macro_rules! define_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, specta::Type)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Generate a new random (v4) id.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Wrap an existing UUID as this id type.
            pub const fn from_uuid(id: Uuid) -> Self {
                Self(id)
            }

            /// The underlying UUID.
            pub const fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s)
                    .map(Self)
                    .map_err(|source| DomainError::InvalidId {
                        type_name: stringify!($name),
                        input: s.to_string(),
                        source,
                    })
            }
        }
    };
}

define_id!(
    ProjectId,
    "Identifies a NACC-managed project (a bound repository plus its NACC-owned configuration)."
);
define_id!(
    WorkflowRunId,
    "Identifies one execution of a workflow template."
);
define_id!(
    RoleProfileId,
    "Identifies one configured Role Matrix row (master plan S11)."
);
define_id!(
    ProviderAccountId,
    "Identifies one configured account profile for a provider (a provider may have several)."
);
define_id!(
    NodeRunId,
    "Identifies one execution of a workflow node within a `WorkflowRun` (master plan S7)."
);
define_id!(
    AttemptId,
    "Identifies one attempt of a `NodeRun` -- a node may be retried, and each retry is its own attempt (master plan S7)."
);
define_id!(
    EventId,
    "Identifies one entry in the durable, normalized event stream (master plan S6, S8.2, S22)."
);
define_id!(
    AuditEventId,
    "Identifies one entry in the audit trail (master plan S7's `AuditEvent`, S22's audit-record fields)."
);

/// A provider identifier. Unlike the UUID-backed IDs above, this is a
/// small, stable, closed enumeration -- new providers are added to NACC's
/// own code, not created by users at runtime, so a UUID would be the wrong
/// shape here. Per master plan S2.7 ("do not hard-code marketing names as
/// architectural constants"), this identifies the *adapter*, not a model:
/// exact model IDs are always provider-reported strings, never hard-coded
/// (see `ModelId` below).
#[derive(
    Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Claude,
    Codex,
    Antigravity,
    Copilot,
    Opencode,
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ProviderId::Claude => "claude",
            ProviderId::Codex => "codex",
            ProviderId::Antigravity => "antigravity",
            ProviderId::Copilot => "copilot",
            ProviderId::Opencode => "opencode",
        };
        f.write_str(s)
    }
}

/// An exact, provider-reported model identifier -- e.g. `"claude-fable-5"`
/// or `"gpt-5.4"`. Deliberately a newtype around `String`, not an enum:
/// master plan S2.7 and S9.5 both require NACC to display exactly what a
/// provider returns rather than assume or hard-code a spelling.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, specta::Type)]
#[serde(transparent)]
pub struct ModelId(pub String);

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ModelId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for ModelId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_of_different_types_have_independent_random_values() {
        let a = ProjectId::new();
        let b = ProjectId::new();
        assert_ne!(a, b, "two freshly generated IDs must not collide");
    }

    #[test]
    fn id_round_trips_through_display_and_from_str() {
        let id = WorkflowRunId::new();
        let s = id.to_string();
        let parsed: WorkflowRunId = s.parse().expect("valid UUID string must parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn id_round_trips_through_json() {
        let id = RoleProfileId::new();
        let json = serde_json::to_string(&id).expect("serialize");
        let back: RoleProfileId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, back);
    }

    #[test]
    fn invalid_id_string_is_a_typed_error_not_a_panic() {
        let result = "not-a-uuid".parse::<ProjectId>();
        assert!(matches!(result, Err(DomainError::InvalidId { .. })));
    }

    #[test]
    fn provider_ids_have_a_stable_total_order() {
        // `ProviderRegistry` keys on `ProviderId` in a `BTreeMap`, so
        // provider lists must not reshuffle between runs.
        let mut ids = vec![ProviderId::Opencode, ProviderId::Claude, ProviderId::Codex];
        ids.sort();
        assert_eq!(
            ids,
            vec![ProviderId::Claude, ProviderId::Codex, ProviderId::Opencode]
        );
    }

    #[test]
    fn provider_id_display_is_lowercase_stable_string() {
        assert_eq!(ProviderId::Claude.to_string(), "claude");
        assert_eq!(ProviderId::Antigravity.to_string(), "antigravity");
    }

    #[test]
    fn model_id_preserves_exact_provider_reported_string() {
        // Master plan S9.5: "Do not hard-code... assume an exact model
        // spelling. Show exactly what the configured provider returns."
        let m: ModelId = "claude-fable-5".into();
        assert_eq!(m.to_string(), "claude-fable-5");
    }
}

/// Canonical reasoning-effort scale (master plan S10.1). Every provider
/// adapter maps this to whatever it natively supports; a provider that
/// cannot honor a requested level must say so explicitly rather than
/// silently clamp -- see `nacc_provider_core::CapabilitySnapshot`.
///
/// This is switch 4 of the Role Matrix's four independently settable
/// per-role controls (role / model / thinking / reasoning effort).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningLevel {
    Auto,
    Off,
    Minimal,
    Low,
    Medium,
    High,
    ExtraHigh,
    Maximum,
}

impl fmt::Display for ReasoningLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ReasoningLevel::Auto => "auto",
            ReasoningLevel::Off => "off",
            ReasoningLevel::Minimal => "minimal",
            ReasoningLevel::Low => "low",
            ReasoningLevel::Medium => "medium",
            ReasoningLevel::High => "high",
            ReasoningLevel::ExtraHigh => "extra_high",
            ReasoningLevel::Maximum => "maximum",
        };
        f.write_str(s)
    }
}

/// Canonical thinking-mode control (master plan S10.2). Deliberately
/// distinct from `ReasoningLevel` -- switches 3 and 4 of the Role Matrix
/// are orthogonal and must never move each other (mission-critical
/// requirement recorded in the Phase 0 plan addendum).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingMode {
    Auto,
    On,
    Off,
    /// The provider manages this internally and exposes no control; show
    /// "Managed by provider" in the GUI rather than a live toggle.
    ManagedByProvider,
    /// The selected provider/model has no thinking concept at all; the
    /// GUI must disable the control, not merely default it to Off.
    Unsupported,
}

impl fmt::Display for ThinkingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ThinkingMode::Auto => "auto",
            ThinkingMode::On => "on",
            ThinkingMode::Off => "off",
            ThinkingMode::ManagedByProvider => "managed_by_provider",
            ThinkingMode::Unsupported => "unsupported",
        };
        f.write_str(s)
    }
}

/// Permission profile a running agent operates under (master plan S12.1).
/// Enforced by `nacc-policy` before every privileged operation; never
/// decorative.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum PermissionProfile {
    ReadOnly,
    PlanOnly,
    AutonomousWorktree,
    RepositoryMaintainer,
    CiMaintainer,
    ReleaseCandidate,
    /// Per-run, time-limited override. Master plan S12.1: "no ability to
    /// make itself persistent" -- `nacc-policy` must enforce the expiry,
    /// this variant only names the state.
    TemporaryDangerFullAccess,
}

impl PermissionProfile {
    /// Where this profile sits on the privilege ladder. Used to *compare* two
    /// profiles -- never to grant anything: the point of a total order here is
    /// that two independent answers to "what may this agent do?" (a workflow
    /// node's declared profile and the Role Matrix row's configured profile)
    /// can be reduced to one, and the reduction is only ever allowed to make
    /// the result *narrower*.
    pub const fn rank(self) -> u8 {
        match self {
            PermissionProfile::ReadOnly => 0,
            PermissionProfile::PlanOnly => 1,
            PermissionProfile::AutonomousWorktree => 2,
            PermissionProfile::RepositoryMaintainer => 3,
            PermissionProfile::CiMaintainer => 4,
            PermissionProfile::ReleaseCandidate => 5,
            PermissionProfile::TemporaryDangerFullAccess => 6,
        }
    }

    /// The narrower of two profiles. Asymmetric on purpose: callers use it as
    /// "the configuration may restrict what a workflow asked for, and may not
    /// expand it" (master plan S12.1) -- so a role row configured as
    /// `ReadOnly` cannot be widened by a node that declares
    /// `RepositoryMaintainer`, while a node asking for `ReadOnly` stays
    /// read-only even if its role row is configured wider.
    pub const fn narrower_of(self, other: Self) -> Self {
        if self.rank() <= other.rank() {
            self
        } else {
            other
        }
    }
}

impl fmt::Display for PermissionProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            PermissionProfile::ReadOnly => "read_only",
            PermissionProfile::PlanOnly => "plan_only",
            PermissionProfile::AutonomousWorktree => "autonomous_worktree",
            PermissionProfile::RepositoryMaintainer => "repository_maintainer",
            PermissionProfile::CiMaintainer => "ci_maintainer",
            PermissionProfile::ReleaseCandidate => "release_candidate",
            PermissionProfile::TemporaryDangerFullAccess => "temporary_danger_full_access",
        };
        f.write_str(s)
    }
}

/// The role catalog (Phase 0 plan addendum's "locked GUI requirement",
/// binding on every later phase): every row is independently configurable
/// and provider-swappable, and users can add custom roles beyond this
/// built-in list. Deliberately open (`Custom(String)`), unlike the closed,
/// provider-normalized event vocabulary in `nacc-events` -- a role is a
/// user-facing organizational concept, not something adapters must map
/// output onto.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RoleKind {
    BrainMainOrchestrator,
    ArchitectPlanner,
    RepositoryExplorer,
    ExternalResearcher,
    FrontendImplementer,
    BackendImplementer,
    DatabaseMigrationImplementer,
    TestEngineer,
    QaReviewer,
    GeneralCodeReviewer,
    SecurityReviewer,
    AccessibilityUxReviewer,
    PerformanceReviewer,
    DocumentationWriter,
    RefactorMigrationSpecialist,
    CiCdInvestigator,
    Integrator,
    ReleaseManager,
    Custom(String),
}

/// One configured Role Matrix row (master plan S7, S11): the four
/// independently settable switches -- role (`role_kind`), model
/// (`provider_id` + `model_id`), thinking (`thinking_mode`), and reasoning
/// effort (`reasoning_level`) -- plus the permission profile it runs
/// under. `provider_id`/`model_id` are `Option` because a role must be
/// *assignable* without being permanently bound: the Phase 0 plan's
/// binding constraint is "no role is ever hard-wired to one provider,"
/// which an unassigned row (both `None`) represents just as validly as an
/// assigned one. Persisted by `nacc-storage`'s role-profile repository
/// (Phase 2); presented and edited by the Role Matrix GUI (Phase 6).
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct RoleProfile {
    pub id: RoleProfileId,
    pub name: String,
    pub role_kind: RoleKind,
    pub provider_id: Option<ProviderId>,
    pub model_id: Option<ModelId>,
    pub thinking_mode: ThinkingMode,
    pub reasoning_level: ReasoningLevel,
    pub permission_profile: PermissionProfile,
    /// Which of the provider's accounts this row prefers -- a *label the
    /// user typed*, never a credential (master plan S11's "account profile"
    /// and S17.4's multiple-accounts-per-provider). `None` means "the
    /// provider's single native sign-in", which is what every adapter
    /// actually uses today; the field exists so multi-account probing needs
    /// no schema change when adapters gain account selection.
    #[serde(default)]
    pub account_label: Option<String>,
    /// The row's own fallback chain (master plan S11, S14.4): consulted when
    /// the row has no primary provider, *after* the node's own declared
    /// fallbacks. Every real fallback is recorded on the attempt
    /// (`AttemptTrigger::Fallback` + reason), so "what actually ran and why"
    /// is always in the audit trail.
    #[serde(default)]
    pub fallbacks: Vec<NodeFallback>,
    pub enabled: bool,
    pub created_at_millis: u64,
    pub updated_at_millis: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct RoleProfileUpdate {
    pub name: String,
    pub role_kind: RoleKind,
    pub provider_id: Option<ProviderId>,
    pub model_id: Option<ModelId>,
    pub thinking_mode: ThinkingMode,
    pub reasoning_level: ReasoningLevel,
    pub permission_profile: PermissionProfile,
    #[serde(default)]
    pub account_label: Option<String>,
    #[serde(default)]
    pub fallbacks: Vec<NodeFallback>,
    pub enabled: bool,
}

define_id!(
    ApprovalId,
    "Identifies one approval request/decision attached to a workflow node (master plan S12, S14.1)."
);
define_id!(
    CapabilitySnapshotId,
    "Identifies one persisted capability snapshot (master plan S4.4's provider-installation and discovered-model data group)."
);
define_id!(
    WorktreeLeaseId,
    "Identifies one NACC-allocated Git worktree lease (master plan S16's worktree lifecycle)."
);

/// Lifecycle state of a worktree lease (master plan S16: allocation,
/// integration, release, quarantine). Closed and exhaustive on purpose: the
/// whole point of the lease record is that "what happened to this
/// worktree?" always has exactly one answer.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeState {
    // Allocated to a run and usable now.
    Active,
    // Moved aside because it was dirty/unpushed when its run ended. Never
    // deleted: master plan S16 requires quarantine over destruction so a
    // human can still recover the work.
    Quarantined,
    // Cleanly removed (its branch was integrated or explicitly abandoned).
    Released,
}

/// One lease on a Git worktree NACC created. Persisted (so it survives a
/// crash and can be reconciled on startup, master plan S16/S17), unlike the
/// raw `git worktree list` output, which only says what exists *right now*
/// and nothing about who owns it or why.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct WorktreeLease {
    pub id: WorktreeLeaseId,
    pub project_id: ProjectId,
    pub workflow_run_id: Option<WorkflowRunId>,
    pub node_run_id: Option<NodeRunId>,
    /// Absolute path of the allocated worktree.
    pub path: String,
    /// The branch checked out in it.
    pub branch: String,
    /// The commit the worktree was created from -- the reference point
    /// drift detection compares against.
    pub base_commit: String,
    /// Last commit NACC observed in this worktree (updated by
    /// `inspect`), so "an agent committed something while we were not
    /// looking" is visible after a restart.
    pub head_commit: Option<String>,
    pub state: WorktreeState,
    /// The process that owned this lease when it was allocated, used by
    /// startup reconciliation to tell "still running" from "orphaned by a
    /// crash". `None` for leases allocated outside a process (tests,
    /// future non-process flows).
    pub owner_process_id: Option<u32>,
    pub quarantine_reason: Option<String>,
    pub created_at_millis: u64,
    pub updated_at_millis: u64,
}

/// Lifecycle state of one workflow run (master plan S14.1's durable state
/// machine). Every variant is terminal-safe: `Paused` and `AwaitingApproval`
/// mean "resumable", and `Interrupted` is the state a crash leaves behind
/// for reconciliation to pick up.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Pending,
    Running,
    /// Deliberately stopped by a user; resumable.
    Paused,
    /// Blocked on a human approval gate (master plan S12.2).
    AwaitingApproval,
    /// The process died mid-run. Reconciliation turns this back into a
    /// resumable run rather than pretending it failed.
    Interrupted,
    Succeeded,
    Failed,
    Cancelled,
}

impl RunState {
    /// Whether the run has stopped moving on its own.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RunState::Succeeded | RunState::Failed | RunState::Cancelled
        )
    }

    /// Whether a caller may resume it.
    pub fn is_resumable(&self) -> bool {
        matches!(
            self,
            RunState::Pending | RunState::Paused | RunState::Interrupted
        )
    }
}

/// State of one node within a run. Closed, like every other state machine
/// here: the engine decides transitions, and a caller that sees an
/// unfamiliar value has a version mismatch to fix rather than a case to
/// guess at.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    /// Not yet eligible (dependencies incomplete).
    Pending,
    Running,
    Succeeded,
    /// Exhausted its attempts, or failed non-retryably.
    Failed,
    /// A dependency failed, so this node will never run.
    Skipped,
    Cancelled,
}

impl NodeState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            NodeState::Succeeded | NodeState::Failed | NodeState::Skipped | NodeState::Cancelled
        )
    }
}

/// Why an attempt happened. Kept separate from the attempt's result because
/// "this was a repair after an independent review finding" and "this was a
/// plain retry after a timeout" are different facts in the audit trail
/// (master plan S14.5).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AttemptTrigger {
    Initial,
    Retry,
    Repair,
    /// A declared fallback provider/role was used because the primary one
    /// was unavailable (master plan S14.5's "visible fallbacks").
    Fallback,
    /// Re-queued by crash recovery.
    Recovery,
}

/// A node that can be re-run, in order, when the primary assignment cannot
/// serve the request. Empty means "no fallback declared" -- which is a real
/// answer, not a missing one: the run then fails visibly instead of
/// silently switching providers.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize, specta::Type)]
pub struct NodeFallback {
    pub provider_id: ProviderId,
    pub model_id: Option<ModelId>,
    /// Shown to the user when the fallback is actually taken.
    pub reason: String,
}

/// A deterministic command that a workflow node declares as completion
/// evidence. This is intentionally a declarative domain contract rather than
/// an executable policy: `nacc-quality` owns spawning and policy checks while
/// the orchestrator owns when declared gates become part of node completion.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize, specta::Type)]
pub struct QualityGateSpec {
    /// Stable within one node and used to correlate durable evidence.
    pub name: String,
    /// Exact executable plus arguments. This is never a shell command string.
    pub argv: Vec<String>,
    /// Per-gate execution ceiling in seconds. `u32` stays IPC-safe for specta.
    pub timeout_secs: u32,
    /// Required gates will eventually participate in the node completion
    /// predicate; optional gates remain evidence without blocking completion.
    pub required: bool,
}

/// One node of a workflow template (master plan S14.2's DAG).
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize, specta::Type)]
pub struct WorkflowNode {
    /// Stable within a template; what `depends_on` and the UI refer to.
    pub key: String,
    pub title: String,
    pub role: RoleKind,
    /// Node keys that must succeed before this node runs.
    pub depends_on: Vec<String>,
    /// Prompt/instruction handed to the role's agent.
    pub instruction: String,
    /// The permission profile this node runs under. Declared per node rather
    /// than per run because master plan S12.1's profiles are per-operation:
    /// an integrator step writes to the repository while the explorer steps
    /// beside it must not.
    pub permission_profile_hint: PermissionProfile,
    /// Never retried automatically when false (e.g. a destructive step).
    pub retryable: bool,
    /// Requires a recorded human approval before it runs (S12.2's
    /// always-approval-gated operations).
    pub requires_approval: bool,
    /// Ceiling for one attempt of this node, in seconds. `None` means the
    /// executor's own default applies -- the declaration is per node because
    /// a quick review step and a long implementation step have genuinely
    /// different natural durations (master plan S14's node contracts). `u32`
    /// because specta refuses pointer-width integers across IPC.
    #[serde(default)]
    pub timeout_secs: Option<u32>,
    /// Machine-readable deterministic checks declared by this node. Older
    /// persisted templates predate this field, so absence must remain exactly
    /// equivalent to declaring no quality gates.
    #[serde(default)]
    pub quality_gates: Vec<QualityGateSpec>,
    pub fallbacks: Vec<NodeFallback>,
}

/// A named DAG, before it is instantiated as a run. Master plan S18's
/// presets are built from this type.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize, specta::Type)]
pub struct WorkflowTemplate {
    pub name: String,
    pub description: String,
    pub nodes: Vec<WorkflowNode>,
}

/// One approval gate (master plan S12.2's "always approval-gated
/// operations"): requested when a node with `requires_approval` becomes
/// ready, decided by a human, and never auto-approved by the engine.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize, specta::Type)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved { by: String },
    Rejected { by: String, reason: String },
}

#[cfg(test)]
mod canonical_control_tests {
    use super::*;

    #[test]
    fn reasoning_level_json_roundtrips_as_snake_case() {
        let json = serde_json::to_string(&ReasoningLevel::ExtraHigh).unwrap();
        assert_eq!(json, "\"extra_high\"");
        let back: ReasoningLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ReasoningLevel::ExtraHigh);
    }

    #[test]
    fn thinking_mode_distinguishes_unsupported_from_off() {
        // These must never collapse into each other: Off is a live user
        // choice, Unsupported means the GUI must disable the control.
        assert_ne!(ThinkingMode::Off, ThinkingMode::Unsupported);
    }

    #[test]
    fn narrowing_a_profile_can_never_widen_it() {
        use PermissionProfile::*;
        let all = [
            ReadOnly,
            PlanOnly,
            AutonomousWorktree,
            RepositoryMaintainer,
            CiMaintainer,
            ReleaseCandidate,
            TemporaryDangerFullAccess,
        ];
        for a in all {
            for b in all {
                let narrowed = a.narrower_of(b);
                assert_eq!(narrowed, b.narrower_of(a), "narrowing must be symmetric");
                assert!(
                    narrowed.rank() <= a.rank() && narrowed.rank() <= b.rank(),
                    "{a} narrowed with {b} produced {narrowed}, which is wider than an input"
                );
            }
        }
        // The two cases the rule actually exists for: a restrictive role row
        // wins over an expansive node declaration, and a restrictive node
        // declaration wins over an expansive role row.
        assert_eq!(RepositoryMaintainer.narrower_of(ReadOnly), ReadOnly);
        assert_eq!(ReadOnly.narrower_of(RepositoryMaintainer), ReadOnly);
    }

    #[test]
    fn permission_profile_ranks_are_strictly_ordered() {
        assert!(PermissionProfile::ReadOnly.rank() < PermissionProfile::PlanOnly.rank());
        assert!(PermissionProfile::PlanOnly.rank() < PermissionProfile::AutonomousWorktree.rank());
        assert!(
            PermissionProfile::AutonomousWorktree.rank()
                < PermissionProfile::RepositoryMaintainer.rank()
        );
        assert!(
            PermissionProfile::RepositoryMaintainer.rank() < PermissionProfile::CiMaintainer.rank()
        );
        assert!(
            PermissionProfile::CiMaintainer.rank() < PermissionProfile::ReleaseCandidate.rank()
        );
        assert!(
            PermissionProfile::ReleaseCandidate.rank()
                < PermissionProfile::TemporaryDangerFullAccess.rank()
        );
    }

    #[test]
    fn permission_profile_json_roundtrips() {
        for p in [
            PermissionProfile::ReadOnly,
            PermissionProfile::PlanOnly,
            PermissionProfile::AutonomousWorktree,
            PermissionProfile::RepositoryMaintainer,
            PermissionProfile::CiMaintainer,
            PermissionProfile::ReleaseCandidate,
            PermissionProfile::TemporaryDangerFullAccess,
        ] {
            let json = serde_json::to_string(&p).unwrap();
            let back: PermissionProfile = serde_json::from_str(&json).unwrap();
            assert_eq!(p, back);
        }
    }

    #[test]
    fn role_kind_builtin_variant_json_is_snake_case() {
        let json = serde_json::to_string(&RoleKind::SecurityReviewer).unwrap();
        assert_eq!(json, "\"security_reviewer\"");
    }

    #[test]
    fn role_kind_custom_variant_roundtrips_the_users_own_name() {
        // Phase 0 plan addendum: "users can add custom roles" -- the
        // catalog above must never be the only option.
        let kind = RoleKind::Custom("Localization Specialist".to_string());
        let json = serde_json::to_string(&kind).unwrap();
        let back: RoleKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, kind);
    }

    #[test]
    fn run_state_terminality_and_resumability_are_distinct_questions() {
        assert!(RunState::Succeeded.is_terminal());
        assert!(RunState::Cancelled.is_terminal());
        assert!(RunState::Paused.is_resumable());
        assert!(RunState::Interrupted.is_resumable());
        // A paused run is not terminal (it can still be resumed), and a
        // terminal run is not resumable -- the two must not be conflated.
        assert!(!RunState::Paused.is_terminal());
        assert!(!RunState::Succeeded.is_resumable());
        assert!(!RunState::AwaitingApproval.is_terminal());
    }

    #[test]
    fn node_state_terminality_includes_skipped() {
        assert!(NodeState::Skipped.is_terminal());
        assert!(!NodeState::Pending.is_terminal());
        assert!(!NodeState::Running.is_terminal());
    }

    #[test]
    fn approval_decision_roundtrips_with_its_decision_tag() {
        let approved = ApprovalDecision::Approved {
            by: "local_user".into(),
        };
        let json = serde_json::to_string(&approved).unwrap();
        assert!(json.contains("\"decision\":\"approved\""));
        let back: ApprovalDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(back, approved);
    }

    #[test]
    fn workflow_template_roundtrips_with_dependencies_and_fallbacks() {
        let template = WorkflowTemplate {
            name: "test".into(),
            description: "test template".into(),
            nodes: vec![WorkflowNode {
                key: "explore".into(),
                title: "Explore".into(),
                role: RoleKind::RepositoryExplorer,
                depends_on: vec![],
                instruction: "explore".into(),
                permission_profile_hint: PermissionProfile::ReadOnly,
                retryable: true,
                requires_approval: false,
                timeout_secs: None,
                quality_gates: vec![QualityGateSpec {
                    name: "unit-tests".into(),
                    argv: vec!["cargo".into(), "test".into(), "-p".into(), "example".into()],
                    timeout_secs: 300,
                    required: true,
                }],
                fallbacks: vec![NodeFallback {
                    provider_id: ProviderId::Codex,
                    model_id: None,
                    reason: "primary provider rate limited".into(),
                }],
            }],
        };
        let json = serde_json::to_string(&template).unwrap();
        let back: WorkflowTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(back.nodes.len(), 1);
        assert_eq!(back.nodes[0].fallbacks[0].provider_id, ProviderId::Codex);
        assert_eq!(back.nodes[0].quality_gates[0].name, "unit-tests");
        assert!(back.nodes[0].quality_gates[0].required);
    }

    #[test]
    fn workflow_node_without_quality_gates_deserializes_as_no_gates() {
        let json = r#"{
            "key":"legacy",
            "title":"Legacy node",
            "role":"repository_explorer",
            "depends_on":[],
            "instruction":"inspect",
            "permission_profile_hint":"read_only",
            "retryable":true,
            "requires_approval":false,
            "timeout_secs":null,
            "fallbacks":[]
        }"#;
        let node: WorkflowNode = serde_json::from_str(json).unwrap();
        assert!(node.quality_gates.is_empty());
    }

    #[test]
    fn worktree_state_json_is_snake_case_and_closed() {
        assert_eq!(
            serde_json::to_string(&WorktreeState::Quarantined).unwrap(),
            "\"quarantined\""
        );
    }

    #[test]
    fn worktree_lease_roundtrips_with_an_absent_owner_process() {
        let lease = WorktreeLease {
            id: WorktreeLeaseId::new(),
            project_id: ProjectId::new(),
            workflow_run_id: None,
            node_run_id: None,
            path: "C:\\worktrees\\impl-1".to_string(),
            branch: "nacc/impl-1".to_string(),
            base_commit: "a".repeat(40),
            head_commit: None,
            state: WorktreeState::Active,
            owner_process_id: None,
            quarantine_reason: None,
            created_at_millis: 1,
            updated_at_millis: 1,
        };
        let json = serde_json::to_string(&lease).unwrap();
        let back: WorktreeLease = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, lease.id);
        assert!(back.head_commit.is_none());
        assert!(back.owner_process_id.is_none());
    }

    #[test]
    fn role_profile_can_be_unassigned_without_being_invalid() {
        // Binding Phase 0 constraint: a role must be assignable to any
        // provider at any time, which includes not being assigned to one
        // right now. `provider_id: None, model_id: None` must round-trip
        // cleanly, not be treated as a malformed state.
        let profile = RoleProfile {
            id: RoleProfileId::new(),
            name: "Primary Reviewer".to_string(),
            role_kind: RoleKind::GeneralCodeReviewer,
            provider_id: None,
            model_id: None,
            thinking_mode: ThinkingMode::Auto,
            reasoning_level: ReasoningLevel::Auto,
            permission_profile: PermissionProfile::ReadOnly,
            account_label: None,
            fallbacks: vec![],
            enabled: true,
            created_at_millis: 1_735_000_000_000,
            updated_at_millis: 1_735_000_000_000,
        };
        let json = serde_json::to_string(&profile).unwrap();
        let back: RoleProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, profile.id);
        assert!(back.provider_id.is_none());
        assert!(back.model_id.is_none());
    }
}

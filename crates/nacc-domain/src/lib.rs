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
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, specta::Type)]
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
    pub enabled: bool,
    pub created_at_millis: u64,
    pub updated_at_millis: u64,
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

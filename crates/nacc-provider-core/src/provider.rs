//! The `AgentProvider` contract (master plan S8.1) and its supporting
//! request/response types. Every provider adapter crate
//! (`nacc-provider-claude`, `-codex`, `-antigravity`, `-copilot`,
//! `-opencode`) implements this trait; nothing outside this crate and
//! `nacc-domain` is referenced by the trait signature, so the orchestrator
//! can depend on `Box<dyn AgentProvider>` without knowing which concrete
//! adapter it holds.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use nacc_domain::{
    ModelId, PermissionProfile, ProviderAccountId, ProviderId, ReasoningLevel, ThinkingMode,
};

use crate::capability::{AuthProbe, CapabilitySnapshot, InstallationProbe, ModelDescriptor, RuntimeLocation};
use crate::error::Result;
use crate::events::EventSink;

/// Where and how a provider CLI is being run (master plan S3, S10 --
/// native Windows / WSL2 / Docker). Kept intentionally small in Phase 1;
/// `nacc-runtime` (Phase 3) is the real abstraction this stands in for.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct RuntimeProfile {
    pub location: RuntimeLocation,
    pub working_directory: String,
}

/// One configured account profile for a provider (a provider may have
/// several -- master plan S17.4: support multiple profiles for the same
/// provider).
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct AccountProfile {
    pub id: ProviderAccountId,
    pub provider: ProviderId,
    /// Safely-exposed label only (e.g. a GitHub login) -- never a
    /// credential. See `AgentProvider::probe_authentication` and master
    /// plan S8.4.
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct CapabilityContext {
    pub account: AccountProfile,
    pub runtime: RuntimeProfile,
}

/// What one Role Matrix row (master plan S11) resolves to at run time --
/// the four independently-set switches (role is implicit in which row
/// this came from; model / thinking / reasoning effort are explicit
/// here), plus the account/runtime/permission it will actually execute
/// under.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct ResolvedAgentProfile {
    pub account: AccountProfile,
    pub model: ModelId,
    pub reasoning: ReasoningLevel,
    pub thinking: ThinkingMode,
    pub permission: PermissionProfile,
    pub runtime: RuntimeProfile,
}

/// Result of validating a `ResolvedAgentProfile` against a provider
/// current `CapabilitySnapshot`. Master plan S10.1: block invalid
/// combinations, never silently downgrade -- `supported: false` must
/// block the run from starting; `issues` is shown verbatim in the GUI
/// effective-settings preview, never swallowed.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct ProfileValidation {
    pub supported: bool,
    pub issues: Vec<String>,
}

/// A provider-native, opaque session identifier -- whatever shape the CLI
/// itself uses (a UUID for Claude, an arbitrary string for others). Not
/// to be confused with NACC own `AgentSession` tracking entity (master
/// plan S7), which is Phase 2 domain-model scope and carries this as one
/// of its fields once it exists.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, specta::Type)]
#[serde(transparent)]
pub struct SessionId(pub String);

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct AgentSessionHandle {
    pub session_id: SessionId,
    pub provider: ProviderId,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentInput {
    Text { text: String },
}

/// Master plan S13.4: on cancellation, request graceful provider
/// shutdown, then terminate the contained tree after a policy-controlled
/// timeout. These two variants are that two-step policy made explicit in
/// the contract, not left to adapter-specific convention.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CancellationMode {
    Graceful,
    Forced,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct LaunchRequest {
    pub profile: ResolvedAgentProfile,
    pub working_directory: String,
    pub prompt: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct ResumeRequest {
    pub session_id: SessionId,
    pub profile: ResolvedAgentProfile,
}

/// Master plan S17.13: distinguish exact provider-reported usage, locally
/// estimated usage, subscription-session counts, API cost, and unknown
/// values. Modeled as a closed set so a UI can never accidentally render
/// an estimate as if it were exact.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
#[serde(tag = "confidence", rename_all = "snake_case")]
pub enum UsageObservation {
    Exact { detail: String },
    Estimated { detail: String },
    Unknown,
}

/// The contract every provider adapter implements (master plan S8.1).
/// Async methods use `async-trait` (rather than native async-fn-in-trait)
/// specifically so `Box<dyn AgentProvider>` works for a heterogeneous
/// provider registry -- native AFIT is not yet dyn-compatible.
#[async_trait]
pub trait AgentProvider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn display_name(&self) -> &str;

    async fn probe_installation(&self, runtime: &RuntimeProfile) -> Result<InstallationProbe>;

    /// Never returns or logs the credential itself (master plan S8.4) --
    /// only whether it is currently valid, and a safely-exposed label.
    async fn probe_authentication(&self, account: &AccountProfile) -> Result<AuthProbe>;

    async fn list_models(&self, account: &AccountProfile) -> Result<Vec<ModelDescriptor>>;

    async fn capabilities(&self, context: &CapabilityContext) -> Result<CapabilitySnapshot>;

    async fn validate_profile(&self, profile: &ResolvedAgentProfile) -> Result<ProfileValidation>;

    async fn launch(
        &self,
        request: LaunchRequest,
        sink: Box<dyn EventSink>,
    ) -> Result<AgentSessionHandle>;

    async fn send_input(&self, session: &SessionId, input: AgentInput) -> Result<()>;

    async fn cancel(&self, session: &SessionId, mode: CancellationMode) -> Result<()>;

    async fn resume(
        &self,
        request: ResumeRequest,
        sink: Box<dyn EventSink>,
    ) -> Result<AgentSessionHandle>;

    async fn collect_usage(&self, session: &SessionId) -> Result<Option<UsageObservation>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ProviderError;

    /// A minimal, deliberately-fake adapter proving the trait is
    /// dyn-compatible and every method signature actually compiles and
    /// is callable through `Box<dyn AgentProvider>` -- exactly the shape
    /// the orchestrator will hold a heterogeneous registry of.
    struct FakeProvider;

    #[async_trait]
    impl AgentProvider for FakeProvider {
        fn id(&self) -> ProviderId {
            ProviderId::Codex
        }
        fn display_name(&self) -> &str {
            "Fake Provider (test double)"
        }
        async fn probe_installation(&self, _runtime: &RuntimeProfile) -> Result<InstallationProbe> {
            Ok(InstallationProbe { installed: true, executable_path: None, version: Some("0.0.0-fake".into()) })
        }
        async fn probe_authentication(&self, _account: &AccountProfile) -> Result<AuthProbe> {
            Ok(AuthProbe { authenticated: true, account_label: Some("fake-user".into()), detail: None })
        }
        async fn list_models(&self, _account: &AccountProfile) -> Result<Vec<ModelDescriptor>> {
            Ok(vec![])
        }
        async fn capabilities(&self, _context: &CapabilityContext) -> Result<CapabilitySnapshot> {
            Err(ProviderError::Other("not implemented in test double".into()))
        }
        async fn validate_profile(&self, _profile: &ResolvedAgentProfile) -> Result<ProfileValidation> {
            Ok(ProfileValidation { supported: true, issues: vec![] })
        }
        async fn launch(&self, _request: LaunchRequest, sink: Box<dyn EventSink>) -> Result<AgentSessionHandle> {
            sink.emit(crate::events::ProviderEvent::SessionCompleted);
            Ok(AgentSessionHandle { session_id: SessionId("fake-session".into()), provider: self.id() })
        }
        async fn send_input(&self, _session: &SessionId, _input: AgentInput) -> Result<()> {
            Ok(())
        }
        async fn cancel(&self, _session: &SessionId, _mode: CancellationMode) -> Result<()> {
            Ok(())
        }
        async fn resume(&self, _request: ResumeRequest, _sink: Box<dyn EventSink>) -> Result<AgentSessionHandle> {
            Err(ProviderError::UnsupportedSetting { detail: "resume not implemented in test double".into() })
        }
        async fn collect_usage(&self, _session: &SessionId) -> Result<Option<UsageObservation>> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn agent_provider_trait_is_dyn_compatible_and_callable() {
        let provider: Box<dyn AgentProvider> = Box::new(FakeProvider);
        assert_eq!(provider.id(), ProviderId::Codex);

        let probe = provider
            .probe_installation(&RuntimeProfile {
                location: RuntimeLocation::NativeWindows,
                working_directory: ".".into(),
            })
            .await
            .unwrap();
        assert!(probe.installed);

        struct NullSink;
        impl EventSink for NullSink {
            fn emit(&self, _event: crate::events::ProviderEvent) {}
        }

        let handle = provider
            .launch(
                LaunchRequest {
                    profile: ResolvedAgentProfile {
                        account: AccountProfile {
                            id: nacc_domain::ProviderAccountId::new(),
                            provider: ProviderId::Codex,
                            label: "fake-user".into(),
                        },
                        model: "fake-model".into(),
                        reasoning: ReasoningLevel::Medium,
                        thinking: ThinkingMode::Unsupported,
                        permission: PermissionProfile::ReadOnly,
                        runtime: RuntimeProfile {
                            location: RuntimeLocation::NativeWindows,
                            working_directory: ".".into(),
                        },
                    },
                    working_directory: ".".into(),
                    prompt: "test".into(),
                },
                Box::new(NullSink),
            )
            .await
            .unwrap();
        assert_eq!(handle.session_id.0, "fake-session");
    }
}

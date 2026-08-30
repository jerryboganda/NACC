//! Provider adapter for Google Antigravity (master plan S9.3).
//!
//! Phase 1 establishes this crate as a real, compiling implementor of
//! `AgentProvider` -- proving the trait boundary is genuinely satisfiable
//! -- without any real CLI-invocation logic. Unlike Claude and Codex
//! (Phase 5 scope), this adapter is explicitly Phase 8 scope, and Phase 0
//! found a real blocker worth recording here: only the Antigravity IDE is
//! installed on the audited machine (an Electron/VSCode-fork GUI
//! application); no standalone headless `agy` CLI exists anywhere on that
//! machine. Live testing also found Google has sunset free-tier
//! individual Gemini CLI access in favor of Antigravity, which raises the
//! stakes on this gap rather than lowering it. See
//! `docs/provider-adapters/antigravity.md` for the full evidence trail,
//! including a legitimate third-party bridge (`codex-router`) flagged as
//! the first thing to evaluate before assuming an official `agy` CLI will
//! appear.

use async_trait::async_trait;

use nacc_domain::ProviderId;
use nacc_provider_core::{
    AccountProfile, AgentInput, AgentProvider, AgentSessionHandle, AuthProbe, CancellationMode,
    CapabilityContext, CapabilitySnapshot, EventSink, InstallationProbe, LaunchRequest,
    ModelDescriptor, ProfileValidation, ProviderError, ResolvedAgentProfile, Result, ResumeRequest,
    RuntimeProfile, SessionId, UsageObservation,
};

#[derive(Default)]
pub struct AntigravityProvider;

fn not_yet_implemented(method: &str) -> ProviderError {
    ProviderError::Other(format!(
        "nacc-provider-antigravity::{method} is not implemented yet -- Phase 8 scope, and blocked on finding a real headless interface (see docs/provider-adapters/antigravity.md)"
    ))
}

#[async_trait]
impl AgentProvider for AntigravityProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Antigravity
    }

    fn display_name(&self) -> &str {
        "Google Antigravity"
    }

    async fn probe_installation(&self, _runtime: &RuntimeProfile) -> Result<InstallationProbe> {
        Err(not_yet_implemented("probe_installation"))
    }

    async fn probe_authentication(&self, _account: &AccountProfile) -> Result<AuthProbe> {
        Err(not_yet_implemented("probe_authentication"))
    }

    async fn list_models(&self, _account: &AccountProfile) -> Result<Vec<ModelDescriptor>> {
        Err(not_yet_implemented("list_models"))
    }

    async fn capabilities(&self, _context: &CapabilityContext) -> Result<CapabilitySnapshot> {
        Err(not_yet_implemented("capabilities"))
    }

    async fn validate_profile(&self, _profile: &ResolvedAgentProfile) -> Result<ProfileValidation> {
        Err(not_yet_implemented("validate_profile"))
    }

    async fn launch(
        &self,
        _request: LaunchRequest,
        _sink: Box<dyn EventSink>,
    ) -> Result<AgentSessionHandle> {
        Err(not_yet_implemented("launch"))
    }

    async fn send_input(&self, _session: &SessionId, _input: AgentInput) -> Result<()> {
        Err(not_yet_implemented("send_input"))
    }

    async fn cancel(&self, _session: &SessionId, _mode: CancellationMode) -> Result<()> {
        Err(not_yet_implemented("cancel"))
    }

    async fn resume(
        &self,
        _request: ResumeRequest,
        _sink: Box<dyn EventSink>,
    ) -> Result<AgentSessionHandle> {
        Err(not_yet_implemented("resume"))
    }

    async fn collect_usage(&self, _session: &SessionId) -> Result<Option<UsageObservation>> {
        Err(not_yet_implemented("collect_usage"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_itself_correctly() {
        let provider = AntigravityProvider;
        assert_eq!(provider.id(), ProviderId::Antigravity);
        assert_eq!(provider.display_name(), "Google Antigravity");
    }

    #[tokio::test]
    async fn unimplemented_methods_return_a_typed_error_not_a_panic() {
        let provider = AntigravityProvider;
        let err = provider
            .probe_installation(&RuntimeProfile {
                location: nacc_provider_core::RuntimeLocation::NativeWindows,
                working_directory: ".".into(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Other(_)));
    }
}

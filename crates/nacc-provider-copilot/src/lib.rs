//! Provider adapter for GitHub Copilot CLI (master plan S9.4).
//!
//! Phase 1 establishes this crate as a real, compiling implementor of
//! `AgentProvider` -- proving the trait boundary is genuinely satisfiable
//! -- without any real CLI-invocation logic, which is Phase 5/10 scope
//! (this adapter is explicitly not tied to a single fixed role; the user
//! intends to use it off and on as the orchestrator brain routed to
//! Claude Fable 5 via BYOK). See `docs/provider-adapters/copilot.md` for
//! the verified CLI contract: `--output-format json` (JSONL), a real
//! `--acp` flag, and a `--effort` scale (`none, minimal, low, medium,
//! high, xhigh, max`) that maps 1:1 onto `nacc_domain::ReasoningLevel`
//! with no lossy translation -- confirmed live from the installed
//! binary help output, correcting an earlier docs-only pass that wrongly
//! concluded Copilot had no structured output at all.

use async_trait::async_trait;

use nacc_domain::ProviderId;
use nacc_provider_core::{
    AccountProfile, AgentInput, AgentProvider, AgentSessionHandle, AuthProbe, CancellationMode,
    CapabilityContext, CapabilitySnapshot, EventSink, InstallationProbe, LaunchRequest,
    ModelDescriptor, ProfileValidation, ProviderError, ResolvedAgentProfile, Result, ResumeRequest,
    RuntimeProfile, SessionId, UsageObservation,
};

#[derive(Default)]
pub struct CopilotProvider;

fn not_yet_implemented(method: &str) -> ProviderError {
    ProviderError::Other(format!(
        "nacc-provider-copilot::{method} is not implemented yet -- real CLI invocation is a later phase"
    ))
}

#[async_trait]
impl AgentProvider for CopilotProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Copilot
    }

    fn display_name(&self) -> &str {
        "GitHub Copilot CLI"
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
        let provider = CopilotProvider;
        assert_eq!(provider.id(), ProviderId::Copilot);
        assert_eq!(provider.display_name(), "GitHub Copilot CLI");
    }

    #[tokio::test]
    async fn unimplemented_methods_return_a_typed_error_not_a_panic() {
        let provider = CopilotProvider;
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

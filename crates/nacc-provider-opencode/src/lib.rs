//! Provider adapter for OpenCode (master plan S9.5).
//!
//! Phase 1 establishes this crate as a real, compiling implementor of
//! `AgentProvider` -- proving the trait boundary is genuinely satisfiable
//! -- without any real CLI-invocation logic, which is Phase 8 scope. See
//! `docs/provider-adapters/opencode.md` for a genuinely important gap
//! found in Phase 0: the OpenCode CLI on the audited machine is currently
//! broken (a wrapper script points at a binary that was never installed),
//! so nothing in that document has been verified against a live process
//! and none of it should be treated as ground truth until a working
//! binary is available to test against. This adapter serves TokenRouter,
//! B.AI, DeepSeek-family, GLM-family, and Qwen-family gateway profiles
//! per the master plan (S9.5).

use async_trait::async_trait;

use nacc_domain::ProviderId;
use nacc_provider_core::{
    AccountProfile, AgentInput, AgentProvider, AgentSessionHandle, AuthProbe, CancellationMode,
    CapabilityContext, CapabilitySnapshot, EventSink, InstallationProbe, LaunchRequest,
    ModelDescriptor, ProfileValidation, ProviderError, ResolvedAgentProfile, Result, ResumeRequest,
    RuntimeProfile, SessionId, UsageObservation,
};

#[derive(Default)]
pub struct OpenCodeProvider;

fn not_yet_implemented(method: &str) -> ProviderError {
    ProviderError::Other(format!(
        "nacc-provider-opencode::{method} is not implemented yet -- Phase 8 scope, and blocked on a working local binary to verify against (see docs/provider-adapters/opencode.md)"
    ))
}

#[async_trait]
impl AgentProvider for OpenCodeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Opencode
    }

    fn display_name(&self) -> &str {
        "OpenCode"
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
        let provider = OpenCodeProvider;
        assert_eq!(provider.id(), ProviderId::Opencode);
        assert_eq!(provider.display_name(), "OpenCode");
    }

    #[tokio::test]
    async fn unimplemented_methods_return_a_typed_error_not_a_panic() {
        let provider = OpenCodeProvider;
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

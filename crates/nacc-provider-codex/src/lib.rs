//! Provider adapter for OpenAI Codex CLI (master plan S9.2).
//!
//! Phase 1 establishes this crate as a real, compiling implementor of
//! `AgentProvider` -- proving the trait boundary is genuinely satisfiable
//! -- without any real CLI-invocation logic, which is Phase 5 scope. See
//! `docs/provider-adapters/codex.md` for the verified CLI contract this
//! crate implements against once Phase 5 begins: `codex exec`, `--json`
//! JSONL output, reasoning effort via `-c model_reasoning_effort=...`
//! (no dedicated top-level flag), and no ACP surface at all (confirmed
//! live -- zero matches for `acp` anywhere in the installed CLI help).

use async_trait::async_trait;

use nacc_domain::ProviderId;
use nacc_provider_core::{
    AccountProfile, AgentInput, AgentProvider, AgentSessionHandle, AuthProbe, CancellationMode,
    CapabilityContext, CapabilitySnapshot, EventSink, InstallationProbe, LaunchRequest,
    ModelDescriptor, ProfileValidation, ProviderError, ResolvedAgentProfile, Result,
    ResumeRequest, RuntimeProfile, SessionId, UsageObservation,
};

#[derive(Default)]
pub struct CodexProvider;

fn not_yet_implemented(method: &str) -> ProviderError {
    ProviderError::Other(format!(
        "nacc-provider-codex::{method} is not implemented yet -- real CLI invocation is Phase 5 scope"
    ))
}

#[async_trait]
impl AgentProvider for CodexProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }

    fn display_name(&self) -> &str {
        "OpenAI Codex"
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

    async fn launch(&self, _request: LaunchRequest, _sink: Box<dyn EventSink>) -> Result<AgentSessionHandle> {
        Err(not_yet_implemented("launch"))
    }

    async fn send_input(&self, _session: &SessionId, _input: AgentInput) -> Result<()> {
        Err(not_yet_implemented("send_input"))
    }

    async fn cancel(&self, _session: &SessionId, _mode: CancellationMode) -> Result<()> {
        Err(not_yet_implemented("cancel"))
    }

    async fn resume(&self, _request: ResumeRequest, _sink: Box<dyn EventSink>) -> Result<AgentSessionHandle> {
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
        let provider = CodexProvider;
        assert_eq!(provider.id(), ProviderId::Codex);
        assert_eq!(provider.display_name(), "OpenAI Codex");
    }

    #[tokio::test]
    async fn unimplemented_methods_return_a_typed_error_not_a_panic() {
        let provider = CodexProvider;
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

//! The adapter contract suite: master plan S17's Phase 4 "contract tests",
//! written once here so every adapter (`claude`, `codex`, `antigravity`,
//! `copilot`, `opencode`) is held to the identical bar instead of each
//! crate inventing its own idea of "tested".
//!
//! # What the suite actually asserts
//!
//! Not "the adapter works" -- that depends on a CLI being installed. It
//! asserts the properties that must hold for *any* correct adapter, using
//! whatever the adapter reports and whatever its injected
//! [`crate::cli::CommandRunner`] returns:
//!
//! - identity is stable and non-empty;
//! - an installation probe never claims a path for a CLI it says is absent;
//! - a capability snapshot describes the adapter that produced it, and its
//!   health follows from its own probes (master plan S2.7 -- no
//!   independently stored "ready" flag);
//! - every model in a snapshot is also a model the adapter lists;
//! - **an unsupported model is never silently accepted** (S10.1);
//! - a launch that succeeds emits `SessionStarted` before anything else and
//!   ends with a terminal event;
//! - cancelling an unknown session is a typed error, not `Ok` and not a
//!   panic;
//! - usage is either absent or explicitly labelled with its confidence
//!   (S17.13).
//!
//! A finding is a *contract violation*, not a failure to have a working
//! CLI: an adapter that correctly reports "not installed" produces zero
//! findings. That is what makes the suite runnable in CI on a machine with
//! no provider CLIs at all -- and simultaneously impossible to pass by
//! accident.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use nacc_domain::{ModelId, PermissionProfile, ProviderId, ReasoningLevel, ThinkingMode};

use crate::cli::CommandRunner;
use crate::error::ProviderError;
use crate::events::{EventSink, ProviderEvent};
use crate::health::ProviderHealth;
use crate::provider::{
    AccountProfile, AgentProvider, CancellationMode, CapabilityContext, LaunchRequest,
    ProfileValidation, ResolvedAgentProfile, ResumeRequest, RuntimeProfile, SessionId,
};

/// One contract violation, with enough detail to act on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractFinding {
    pub check: ContractCheck,
    pub detail: String,
}

/// The closed list of contract checks (so a report can be filtered, and so
/// adding a check is a deliberate, visible change).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ContractCheck {
    IdentityStable,
    InstallationProbeConsistent,
    SnapshotDescribesItself,
    SnapshotHealthFollowsProbes,
    SnapshotModelsAreListed,
    UnsupportedModelRejected,
    LaunchEventOrder,
    CancelUnknownSessionIsTyped,
    UsageConfidenceLabelled,
}

impl ContractCheck {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContractCheck::IdentityStable => "identity_stable",
            ContractCheck::InstallationProbeConsistent => "installation_probe_consistent",
            ContractCheck::SnapshotDescribesItself => "snapshot_describes_itself",
            ContractCheck::SnapshotHealthFollowsProbes => "snapshot_health_follows_probes",
            ContractCheck::SnapshotModelsAreListed => "snapshot_models_are_listed",
            ContractCheck::UnsupportedModelRejected => "unsupported_model_rejected",
            ContractCheck::LaunchEventOrder => "launch_event_order",
            ContractCheck::CancelUnknownSessionIsTyped => "cancel_unknown_session_is_typed",
            ContractCheck::UsageConfidenceLabelled => "usage_confidence_labelled",
        }
    }
}

/// Everything the suite needs to drive an adapter without knowing which one
/// it is.
pub struct ContractHarness {
    pub account: AccountProfile,
    pub runtime: RuntimeProfile,
    pub working_directory: String,
    /// Injected so the suite exercises the adapter's real code path against
    /// recorded output rather than a live CLI.
    pub runner: Arc<dyn CommandRunner>,
}

impl ContractHarness {
    pub fn new(provider: ProviderId, runner: Arc<dyn CommandRunner>) -> Self {
        let working_directory = std::env::temp_dir().to_string_lossy().into_owned();
        Self {
            account: AccountProfile {
                id: nacc_domain::ProviderAccountId::new(),
                provider,
                label: "contract-suite".to_string(),
            },
            runtime: RuntimeProfile {
                location: crate::capability::RuntimeLocation::NativeWindows,
                working_directory: working_directory.clone(),
            },
            working_directory,
            runner,
        }
    }

    pub fn context(&self) -> CapabilityContext {
        CapabilityContext {
            account: self.account.clone(),
            runtime: self.runtime.clone(),
        }
    }

    pub fn profile_for(&self, model: ModelId) -> ResolvedAgentProfile {
        ResolvedAgentProfile {
            account: self.account.clone(),
            model,
            reasoning: ReasoningLevel::Auto,
            thinking: ThinkingMode::Auto,
            permission: PermissionProfile::ReadOnly,
            runtime: self.runtime.clone(),
        }
    }
}

/// The result of one suite run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContractReport {
    pub provider: Option<ProviderId>,
    pub findings: Vec<ContractFinding>,
}

impl ContractReport {
    pub fn passed(&self) -> bool {
        self.findings.is_empty()
    }

    /// A failure message that names every violation -- the form an adapter
    /// test uses in its assertion, so a failing CI run says precisely which
    /// property broke.
    pub fn summary(&self) -> String {
        if self.passed() {
            return "adapter contract suite passed".to_string();
        }
        let mut out = format!("{} contract violation(s):", self.findings.len());
        for finding in &self.findings {
            out.push_str(&format!(
                "\n  - [{}] {}",
                finding.check.as_str(),
                finding.detail
            ));
        }
        out
    }
}

/// Records every normalized event an adapter emits, so launch behavior can
/// be asserted rather than assumed. Public because adapter tests use it too.
#[derive(Default)]
pub struct RecordingSink {
    events: Mutex<Vec<ProviderEvent>>,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<ProviderEvent> {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn len(&self) -> usize {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Index of the first event matching `predicate`.
    pub fn position_of(&self, predicate: impl Fn(&ProviderEvent) -> bool) -> Option<usize> {
        self.events().iter().position(predicate)
    }
}

impl EventSink for RecordingSink {
    fn emit(&self, event: ProviderEvent) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(event);
    }
}

/// Run every contract check against `provider`.
pub async fn run_contract_suite(
    provider: &dyn AgentProvider,
    harness: &ContractHarness,
) -> ContractReport {
    let mut report = ContractReport {
        provider: Some(provider.id()),
        findings: Vec::new(),
    };
    let id = provider.id();

    // --- identity -----------------------------------------------------
    let mut finding = |check: ContractCheck, detail: String| {
        report.findings.push(ContractFinding { check, detail });
    };

    if provider.display_name().trim().is_empty() {
        finding(
            ContractCheck::IdentityStable,
            "display_name() is empty".to_string(),
        );
    }
    if provider.id() != id {
        finding(
            ContractCheck::IdentityStable,
            "id() returned a different value on a second call".to_string(),
        );
    }

    // --- installation probe -------------------------------------------
    let installation = match provider.probe_installation(&harness.runtime).await {
        Ok(probe) => Some(probe),
        Err(ProviderError::NotInstalled { .. }) | Err(ProviderError::Other(_)) => None,
        Err(other) => {
            finding(
                ContractCheck::InstallationProbeConsistent,
                format!("probe_installation returned an unexpected error: {other}"),
            );
            None
        }
    };
    if let Some(probe) = &installation {
        if !probe.installed && probe.executable_path.is_some() {
            finding(
                ContractCheck::InstallationProbeConsistent,
                "reported installed=false while also reporting an executable path".to_string(),
            );
        }
        if probe.installed && probe.version.as_deref().unwrap_or("").trim().is_empty() {
            finding(
                ContractCheck::InstallationProbeConsistent,
                "reported installed=true with no version string; the version is what a user is \
                 shown and what a support bundle needs"
                    .to_string(),
            );
        }
    }

    // --- capability snapshot ------------------------------------------
    let snapshot = provider.capabilities(&harness.context()).await.ok();
    if let Some(snapshot) = &snapshot {
        if snapshot.provider != id {
            finding(
                ContractCheck::SnapshotDescribesItself,
                format!(
                    "capabilities() from {} reported provider {:?}",
                    id, snapshot.provider
                ),
            );
        }
        let expected_health = ProviderHealth::from_probes(
            &snapshot.installation,
            &snapshot.auth,
            match &snapshot.health {
                ProviderHealth::IneligibleCredential { detail } => Some(detail.clone()),
                _ => None,
            },
        );
        if expected_health != snapshot.health {
            finding(
                ContractCheck::SnapshotHealthFollowsProbes,
                format!(
                    "health {:?} does not follow from its own probes (expected {expected_health:?})",
                    snapshot.health
                ),
            );
        }

        if let Ok(listed) = provider.list_models(&harness.account).await {
            for model in &snapshot.models {
                if model.id.0.trim().is_empty() {
                    finding(
                        ContractCheck::SnapshotModelsAreListed,
                        "a model descriptor has an empty id".to_string(),
                    );
                    continue;
                }
                if !listed.iter().any(|candidate| candidate.id == model.id) {
                    finding(
                        ContractCheck::SnapshotModelsAreListed,
                        format!(
                            "snapshot advertises model {} which list_models() does not return",
                            model.id
                        ),
                    );
                }
            }
        }
    }

    // --- honesty about unsupported models ------------------------------
    let bogus = ModelId("nacc-contract-nonexistent-model".to_string());
    let known_models_contain_bogus = snapshot
        .as_ref()
        .map(|snapshot| snapshot.models.iter().any(|m| m.id == bogus))
        .unwrap_or(false);
    if !known_models_contain_bogus {
        // `if let` on the one shape that is a violation: every other
        // outcome (explicitly unsupported, or a typed error) is a correct
        // answer to "validate this model" when the model is unknown.
        if let Ok(ProfileValidation {
            supported: true, ..
        }) = provider
            .validate_profile(&harness.profile_for(bogus.clone()))
            .await
        {
            finding(
                ContractCheck::UnsupportedModelRejected,
                format!(
                    "validate_profile accepted {bogus}, which this adapter does not report as a \
                     model; master plan S10.1 forbids silently accepting an unsupported \
                     combination"
                ),
            );
        }
    }

    // --- launch behavior ------------------------------------------------
    let sink = Arc::new(RecordingSink::new());
    struct SinkAdapter(Arc<RecordingSink>);
    impl EventSink for SinkAdapter {
        fn emit(&self, event: ProviderEvent) {
            self.0.emit(event);
        }
    }

    let launch_model = snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.models.first().map(|m| m.id.clone()))
        .unwrap_or_else(|| ModelId("nacc-contract-model".to_string()));
    let launch = provider
        .launch(
            LaunchRequest {
                profile: harness.profile_for(launch_model),
                working_directory: harness.working_directory.clone(),
                prompt: "contract suite probe".to_string(),
            },
            Box::new(SinkAdapter(Arc::clone(&sink))),
        )
        .await;

    match &launch {
        Ok(handle) => {
            if handle.provider != id {
                finding(
                    ContractCheck::LaunchEventOrder,
                    format!("launch returned a handle for {:?}", handle.provider),
                );
            }
            if handle.session_id.0.trim().is_empty() {
                finding(
                    ContractCheck::LaunchEventOrder,
                    "launch returned an empty session id".to_string(),
                );
            }
            let events = sink.events();
            match events.first() {
                Some(ProviderEvent::SessionStarted { .. }) => {}
                Some(other) => finding(
                    ContractCheck::LaunchEventOrder,
                    format!("first emitted event was {other:?}, not SessionStarted"),
                ),
                None => finding(
                    ContractCheck::LaunchEventOrder,
                    "launch returned Ok but emitted no events at all".to_string(),
                ),
            }
            let terminal = events.iter().any(|event| {
                matches!(
                    event,
                    ProviderEvent::SessionCompleted | ProviderEvent::SessionCancelled
                )
            });
            // A long-running launch legitimately has no terminal event yet,
            // but a *synchronous* launch (which is what a fixture-backed
            // probe is) must not claim success while leaving the sink empty
            // of any terminal state *and* any assistant output.
            if !terminal
                && !events
                    .iter()
                    .any(|event| matches!(event, ProviderEvent::AssistantTextDelta { .. }))
            {
                finding(
                    ContractCheck::LaunchEventOrder,
                    "launch returned Ok but emitted neither a terminal event nor any output"
                        .to_string(),
                );
            }
        }
        Err(ProviderError::NotInstalled { .. })
        | Err(ProviderError::Unauthenticated { .. })
        | Err(ProviderError::IneligibleCredential { .. })
        | Err(ProviderError::UnsupportedSetting { .. })
        | Err(ProviderError::Other(_))
        | Err(ProviderError::Process(_)) => {}
        Err(other) => finding(
            ContractCheck::LaunchEventOrder,
            format!("launch failed with an unexpected error: {other}"),
        ),
    }

    // --- cancelling an unknown session ---------------------------------
    if provider
        .cancel(
            &SessionId("nacc-contract-unknown-session".into()),
            CancellationMode::Forced,
        )
        .await
        .is_ok()
    {
        finding(
            ContractCheck::CancelUnknownSessionIsTyped,
            "cancel() of a session that was never launched returned Ok".to_string(),
        );
    }

    // --- usage honesty --------------------------------------------------
    if let Ok(handle) = &launch {
        if let Ok(Some(observation)) = provider.collect_usage(&handle.session_id).await {
            let unlabelled = match &observation {
                crate::provider::UsageObservation::Exact { detail }
                | crate::provider::UsageObservation::Estimated { detail } => {
                    detail.trim().is_empty()
                }
                crate::provider::UsageObservation::Unknown => false,
            };
            if unlabelled {
                finding(
                    ContractCheck::UsageConfidenceLabelled,
                    "a usage observation carries no detail, so its confidence cannot be \
                     checked by a user"
                        .to_string(),
                );
            }
        }
    }

    report
}

/// A resume request built the same way a caller would, for adapters whose
/// suite run wants to exercise `resume` too.
pub fn contract_resume_request(harness: &ContractHarness, model: ModelId) -> ResumeRequest {
    ResumeRequest {
        session_id: SessionId("nacc-contract-unknown-session".into()),
        profile: harness.profile_for(model),
    }
}

/// Directory convention: an adapter's own fixtures live in
/// `<crate>/fixtures`, next to the CLI contract doc that records the exact
/// version they were captured from.
pub fn adapter_fixture_dir(crate_manifest_dir: &str) -> PathBuf {
    PathBuf::from(crate_manifest_dir).join("fixtures")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{
        AcpTransport, AuthProbe, CapabilitySnapshot, InstallationProbe, ModelDescriptor,
    };
    use crate::cli::FixtureCommandRunner;
    use crate::error::Result;
    use crate::provider::{
        AccountProfile, AgentInput, AgentSessionHandle, RuntimeProfile, UsageObservation,
    };
    use crate::RuntimeLocation;
    use async_trait::async_trait;

    /// The contract's executable definition: an adapter that behaves
    /// correctly in every respect the suite checks, with no CLI at all.
    /// Deliberately in-memory rather than fixture-backed, so the suite's
    /// own test does not depend on captured provider output.
    struct ReferenceProvider {
        installed: bool,
    }

    #[async_trait]
    impl AgentProvider for ReferenceProvider {
        fn id(&self) -> ProviderId {
            ProviderId::Claude
        }
        fn display_name(&self) -> &str {
            "Reference Provider"
        }
        async fn probe_installation(&self, _runtime: &RuntimeProfile) -> Result<InstallationProbe> {
            Ok(InstallationProbe {
                installed: self.installed,
                executable_path: self
                    .installed
                    .then(|| "C:\\reference\\claude.exe".to_string()),
                version: self.installed.then(|| "9.9.9".to_string()),
            })
        }
        async fn probe_authentication(&self, _a: &AccountProfile) -> Result<AuthProbe> {
            Ok(AuthProbe {
                authenticated: self.installed,
                account_label: self.installed.then(|| "reference-user".to_string()),
                detail: None,
            })
        }
        async fn list_models(&self, _a: &AccountProfile) -> Result<Vec<ModelDescriptor>> {
            Ok(vec![self.model()])
        }
        async fn capabilities(&self, context: &CapabilityContext) -> Result<CapabilitySnapshot> {
            let installation = self.probe_installation(&context.runtime).await?;
            let auth = self.probe_authentication(&context.account).await?;
            let health = ProviderHealth::from_probes(&installation, &auth, None);
            Ok(CapabilitySnapshot {
                provider: self.id(),
                runtime: RuntimeLocation::NativeWindows,
                health,
                installation,
                auth,
                models: vec![self.model()],
                noninteractive_mode: true,
                structured_json_output: true,
                streaming_json_output: true,
                interactive_pty: false,
                session_resume: true,
                custom_agents: false,
                subagents: false,
                mcp: false,
                acp_transport: AcpTransport::Unsupported,
                usage_reporting: true,
                cancellation_documented: true,
                captured_at_millis: 1_735_000_000_000,
            })
        }
        async fn validate_profile(
            &self,
            profile: &ResolvedAgentProfile,
        ) -> Result<ProfileValidation> {
            if profile.model == self.model().id {
                Ok(ProfileValidation {
                    supported: true,
                    issues: vec![],
                })
            } else {
                Ok(ProfileValidation {
                    supported: false,
                    issues: vec![format!("unknown model {}", profile.model)],
                })
            }
        }
        async fn launch(
            &self,
            request: LaunchRequest,
            sink: Box<dyn EventSink>,
        ) -> Result<AgentSessionHandle> {
            if !self.installed {
                return Err(ProviderError::NotInstalled {
                    detail: "reference provider is not installed in this configuration".into(),
                });
            }
            sink.emit(ProviderEvent::SessionStarted {
                provider_session_id: "reference-session".into(),
                model: request.profile.model,
            });
            sink.emit(ProviderEvent::AssistantTextDelta {
                text: "reference output".into(),
            });
            sink.emit(ProviderEvent::SessionCompleted);
            Ok(AgentSessionHandle {
                session_id: SessionId("reference-session".into()),
                provider: self.id(),
            })
        }
        async fn send_input(&self, _s: &SessionId, _i: AgentInput) -> Result<()> {
            Err(ProviderError::UnsupportedSetting {
                detail: "reference provider has no follow-up input".into(),
            })
        }
        async fn cancel(&self, _s: &SessionId, _m: CancellationMode) -> Result<()> {
            Err(ProviderError::Other("unknown session".into()))
        }
        async fn resume(
            &self,
            _r: ResumeRequest,
            _s: Box<dyn EventSink>,
        ) -> Result<AgentSessionHandle> {
            Err(ProviderError::UnsupportedSetting {
                detail: "reference provider does not resume".into(),
            })
        }
        async fn collect_usage(&self, _s: &SessionId) -> Result<Option<UsageObservation>> {
            Ok(Some(UsageObservation::Estimated {
                detail: "estimate: 1 request".into(),
            }))
        }
    }

    impl ReferenceProvider {
        fn model(&self) -> ModelDescriptor {
            ModelDescriptor {
                id: ModelId("reference-model".into()),
                display_name: "Reference Model".into(),
                reasoning_levels: vec![ReasoningLevel::Auto, ReasoningLevel::High],
                thinking: ThinkingMode::Unsupported,
                structured_output: true,
                context_window_tokens: Some(200_000),
            }
        }
    }

    fn harness() -> ContractHarness {
        ContractHarness::new(
            ProviderId::Claude,
            Arc::new(FixtureCommandRunner::default()),
        )
    }

    #[tokio::test]
    async fn a_correct_adapter_passes_every_check() {
        let provider = ReferenceProvider { installed: true };
        let report = run_contract_suite(&provider, &harness()).await;
        assert!(report.passed(), "{}", report.summary());
        assert_eq!(report.provider, Some(ProviderId::Claude));
    }

    #[tokio::test]
    async fn a_correctly_absent_cli_also_passes() {
        // The suite must be runnable in CI where no provider CLI exists --
        // "honest not-installed reporting" is a pass, not a skip.
        let provider = ReferenceProvider { installed: false };
        let report = run_contract_suite(&provider, &harness()).await;
        assert!(report.passed(), "{}", report.summary());
    }

    // --- the suite can actually fail: each check gets a deliberately
    // --- broken adapter and asserts the matching finding appears.

    struct BrokenProvider {
        broken: &'static str,
    }

    #[async_trait]
    impl AgentProvider for BrokenProvider {
        fn id(&self) -> ProviderId {
            ProviderId::Claude
        }
        fn display_name(&self) -> &str {
            if self.broken == "identity" {
                "   "
            } else {
                "Broken"
            }
        }
        async fn probe_installation(&self, _r: &RuntimeProfile) -> Result<InstallationProbe> {
            Ok(InstallationProbe {
                installed: false,
                executable_path: Some("C:\\phantom\\claude.exe".into()),
                version: None,
            })
        }
        async fn probe_authentication(&self, _a: &AccountProfile) -> Result<AuthProbe> {
            Ok(AuthProbe {
                authenticated: false,
                account_label: None,
                detail: None,
            })
        }
        async fn list_models(&self, _a: &AccountProfile) -> Result<Vec<ModelDescriptor>> {
            Ok(vec![])
        }
        async fn capabilities(&self, _c: &CapabilityContext) -> Result<CapabilitySnapshot> {
            let installation = InstallationProbe {
                installed: false,
                executable_path: None,
                version: None,
            };
            let auth = AuthProbe {
                authenticated: false,
                account_label: None,
                detail: None,
            };
            Ok(CapabilitySnapshot {
                provider: if self.broken == "snapshot" {
                    ProviderId::Codex
                } else {
                    ProviderId::Claude
                },
                runtime: RuntimeLocation::NativeWindows,
                // A snapshot claiming ready while its own probes say
                // otherwise: the exact "GUI shows green for a provider
                // that cannot launch" failure.
                health: if self.broken == "health" {
                    ProviderHealth::Ready
                } else {
                    ProviderHealth::NotInstalled
                },
                installation,
                auth,
                models: vec![ModelDescriptor {
                    id: ModelId("ghost-model".into()),
                    display_name: "Ghost".into(),
                    reasoning_levels: vec![],
                    thinking: ThinkingMode::Unsupported,
                    structured_output: false,
                    context_window_tokens: None,
                }],
                noninteractive_mode: false,
                structured_json_output: false,
                streaming_json_output: false,
                interactive_pty: false,
                session_resume: false,
                custom_agents: false,
                subagents: false,
                mcp: false,
                acp_transport: AcpTransport::Unsupported,
                usage_reporting: false,
                cancellation_documented: false,
                captured_at_millis: 0,
            })
        }
        async fn validate_profile(&self, _p: &ResolvedAgentProfile) -> Result<ProfileValidation> {
            // Accepts anything -- the catastrophic case S10.1 forbids.
            Ok(ProfileValidation {
                supported: true,
                issues: vec![],
            })
        }
        async fn launch(
            &self,
            _r: LaunchRequest,
            sink: Box<dyn EventSink>,
        ) -> Result<AgentSessionHandle> {
            // Emits the terminal event first and no SessionStarted.
            sink.emit(ProviderEvent::SessionCompleted);
            Ok(AgentSessionHandle {
                session_id: SessionId(String::new()),
                provider: ProviderId::Claude,
            })
        }
        async fn send_input(&self, _s: &SessionId, _i: AgentInput) -> Result<()> {
            Ok(())
        }
        async fn cancel(&self, _s: &SessionId, _m: CancellationMode) -> Result<()> {
            // Claims success for a session that does not exist.
            Ok(())
        }
        async fn resume(
            &self,
            _r: ResumeRequest,
            _s: Box<dyn EventSink>,
        ) -> Result<AgentSessionHandle> {
            Err(ProviderError::Other("broken".into()))
        }
        async fn collect_usage(&self, _s: &SessionId) -> Result<Option<UsageObservation>> {
            Ok(Some(UsageObservation::Exact {
                detail: "   ".into(),
            }))
        }
    }

    #[tokio::test]
    async fn the_suite_reports_each_violation_it_is_designed_to_catch() {
        for broken in [
            "identity",
            "installation",
            "snapshot",
            "health",
            "models",
            "honesty",
            "launch",
            "cancel",
            "usage",
        ] {
            let provider = BrokenProvider { broken };
            let report = run_contract_suite(&provider, &harness()).await;
            assert!(
                !report.passed(),
                "the suite must not pass a deliberately broken adapter (case {broken})"
            );
            let expected = match broken {
                "identity" => ContractCheck::IdentityStable,
                "installation" => ContractCheck::InstallationProbeConsistent,
                "snapshot" => ContractCheck::SnapshotDescribesItself,
                "health" => ContractCheck::SnapshotHealthFollowsProbes,
                "models" => ContractCheck::SnapshotModelsAreListed,
                "honesty" => ContractCheck::UnsupportedModelRejected,
                "launch" => ContractCheck::LaunchEventOrder,
                "cancel" => ContractCheck::CancelUnknownSessionIsTyped,
                "usage" => ContractCheck::UsageConfidenceLabelled,
                other => panic!("unhandled case {other}"),
            };
            assert!(
                report.findings.iter().any(|f| f.check == expected),
                "case {broken} must produce a {expected:?} finding; got {}",
                report.summary()
            );
        }
    }

    #[test]
    fn a_passing_report_explains_itself() {
        let report = ContractReport {
            provider: Some(ProviderId::Codex),
            findings: vec![],
        };
        assert_eq!(report.summary(), "adapter contract suite passed");
    }

    #[test]
    fn a_failing_report_names_every_violation() {
        let report = ContractReport {
            provider: Some(ProviderId::Codex),
            findings: vec![
                ContractFinding {
                    check: ContractCheck::IdentityStable,
                    detail: "empty display name".into(),
                },
                ContractFinding {
                    check: ContractCheck::UnsupportedModelRejected,
                    detail: "accepted a bogus model".into(),
                },
            ],
        };
        let summary = report.summary();
        assert!(summary.contains("identity_stable"));
        assert!(summary.contains("accepted a bogus model"));
    }

    #[test]
    fn recording_sink_exposes_event_order() {
        let sink = RecordingSink::new();
        sink.emit(ProviderEvent::SessionStarted {
            provider_session_id: "s".into(),
            model: ModelId("m".into()),
        });
        sink.emit(ProviderEvent::SessionCompleted);
        assert_eq!(
            sink.position_of(|event| matches!(event, ProviderEvent::SessionCompleted)),
            Some(1)
        );
        assert!(sink
            .position_of(|event| matches!(event, ProviderEvent::Warning { .. }))
            .is_none());
    }

    #[tokio::test]
    async fn resume_helper_builds_a_request_for_an_unknown_session() {
        let request = contract_resume_request(&harness(), ModelId("m".into()));
        assert_eq!(request.session_id.0, "nacc-contract-unknown-session");
    }
}

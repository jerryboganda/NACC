//! Codex CLI adapter tests: the documented mappings (exact), JSONL
//! interpretation against documented-shape fixtures, the adapter contract
//! suite, and an end-to-end launch through `FixtureCommandRunner`.
//!
//! See `fixtures/README.md` for what is captured versus documented-shape.
//! No test here claims a live `--json` stream was verified; the interpreter
//! is instead asserted to *ignore* vocabulary it does not recognize, which is
//! the property that keeps an unverified stream shape from becoming an
//! outage.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use nacc_provider_core::{
    run_contract_suite, CommandLineSink, ContractHarness, EventSink, FixtureCommandRunner,
    RecordedInvocation, RecordingSink,
};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn load(name: &str) -> Vec<RecordedInvocation> {
    serde_json::from_slice(&std::fs::read(fixture_dir().join(name)).unwrap())
        .unwrap_or_else(|err| panic!("fixture {name} must be valid JSON: {err}"))
}

fn jsonl_runner() -> Arc<FixtureCommandRunner> {
    Arc::new(FixtureCommandRunner::new(load("jsonl-run.json")))
}

fn version_runner() -> Arc<FixtureCommandRunner> {
    Arc::new(FixtureCommandRunner::new(load("invocations.json")))
}

fn profile(
    model: &str,
    reasoning: ReasoningLevel,
    permission: PermissionProfile,
) -> ResolvedAgentProfile {
    ResolvedAgentProfile {
        account: AccountProfile {
            id: nacc_domain::ProviderAccountId::new(),
            provider: ProviderId::Codex,
            label: "test-account".into(),
        },
        model: model.into(),
        reasoning,
        thinking: ThinkingMode::Auto,
        permission,
        runtime: RuntimeProfile {
            location: nacc_provider_core::RuntimeLocation::NativeWindows,
            working_directory: std::env::temp_dir().to_string_lossy().into_owned(),
        },
    }
}

fn session() -> SessionId {
    SessionId(nacc_domain::WorkflowRunId::new().to_string())
}

struct SharingSink(Arc<RecordingSink>);

impl EventSink for SharingSink {
    fn emit(&self, event: ProviderEvent) {
        self.0.emit(event);
    }
}

// --- documented mappings -------------------------------------------------

#[test]
fn reasoning_mapping_matches_the_documented_config_vocabulary() {
    // `minimal|low|medium|high|xhigh` only: no `none`, no `max`.
    for (level, expected) in [
        (ReasoningLevel::Auto, (None, true)),
        (ReasoningLevel::Off, (Some("minimal"), false)),
        (ReasoningLevel::Minimal, (Some("minimal"), false)),
        (ReasoningLevel::Low, (Some("low"), true)),
        (ReasoningLevel::Medium, (Some("medium"), true)),
        (ReasoningLevel::High, (Some("high"), true)),
        (ReasoningLevel::ExtraHigh, (Some("xhigh"), true)),
        (ReasoningLevel::Maximum, (Some("xhigh"), false)),
    ] {
        assert_eq!(map_reasoning(level), expected, "mapping for {level}");
    }
    assert!(reasoning_mapping_description(ReasoningLevel::Maximum).contains("clamped"));
}

#[test]
fn sandbox_modes_are_exactly_the_documented_set() {
    for (profile_kind, expected) in [
        (PermissionProfile::ReadOnly, "read-only"),
        (PermissionProfile::PlanOnly, "read-only"),
        (PermissionProfile::AutonomousWorktree, "workspace-write"),
        (PermissionProfile::RepositoryMaintainer, "workspace-write"),
        (PermissionProfile::CiMaintainer, "workspace-write"),
        (PermissionProfile::ReleaseCandidate, "workspace-write"),
        (
            PermissionProfile::TemporaryDangerFullAccess,
            "danger-full-access",
        ),
    ] {
        assert_eq!(sandbox_mode(profile_kind), expected);
    }
}

#[test]
fn the_never_used_bypass_flag_cannot_be_reached_from_any_profile() {
    // Phase 0 recorded a real open issue where the bypass flag can stall
    // forever, and master plan S12.2 requires approval gating. Only the
    // explicitly temporary dangerous profile opens the sandbox, and even
    // that one keeps approval on-request.
    for profile_kind in [
        PermissionProfile::ReadOnly,
        PermissionProfile::PlanOnly,
        PermissionProfile::AutonomousWorktree,
        PermissionProfile::RepositoryMaintainer,
        PermissionProfile::CiMaintainer,
        PermissionProfile::ReleaseCandidate,
    ] {
        let args = build_launch_args(
            &profile("gpt-5-codex", ReasoningLevel::High, profile_kind),
            "p",
        );
        assert!(
            !args.iter().any(|arg| arg.contains("dangerously-bypass")),
            "{profile_kind:?} must not produce the bypass flag: {args:?}"
        );
        // Approval is only pre-answered for the non-mutating profiles; every
        // mutating profile must ask.
        match profile_kind {
            PermissionProfile::ReadOnly | PermissionProfile::PlanOnly => {
                assert_eq!(approval_policy(profile_kind), "never")
            }
            _ => assert_eq!(approval_policy(profile_kind), "on-request"),
        }
    }
    let dangerous = build_launch_args(
        &profile(
            "gpt-5-codex",
            ReasoningLevel::High,
            PermissionProfile::TemporaryDangerFullAccess,
        ),
        "p",
    );
    assert!(dangerous.contains(&"danger-full-access".to_string()));
    assert!(
        !dangerous
            .iter()
            .any(|arg| arg.contains("dangerously-bypass")),
        "even the dangerous profile keeps approvals on-request: {dangerous:?}"
    );
    assert!(dangerous.contains(&"on-request".to_string()));
}

#[test]
fn launch_args_use_exec_json_and_no_deprecated_full_auto_flag() {
    let args = build_launch_args(
        &profile(
            "gpt-5-codex",
            ReasoningLevel::High,
            PermissionProfile::AutonomousWorktree,
        ),
        "fix it",
    );
    assert_eq!(
        args,
        vec![
            "exec",
            "--json",
            "-m",
            "gpt-5-codex",
            "-s",
            "workspace-write",
            "-a",
            "on-request",
            "-c",
            "model_reasoning_effort=high",
            "fix it",
        ]
    );
    assert!(
        !args.iter().any(|arg| arg == "--full-auto"),
        "`--full-auto` is deprecated and absent from the installed CLI"
    );
}

#[test]
fn resume_args_use_the_documented_exec_resume_subcommand() {
    let args = build_resume_args(
        &profile(
            "gpt-5.4",
            ReasoningLevel::Medium,
            PermissionProfile::ReadOnly,
        ),
        "0192f0aa-1111-7222-8333-444455556666",
    );
    assert_eq!(args[0], "exec");
    assert!(args.contains(&"resume".to_string()));
    assert!(args.contains(&"0192f0aa-1111-7222-8333-444455556666".to_string()));
}

// --- JSONL interpretation -----------------------------------------------

#[test]
fn session_creation_is_how_the_provider_id_is_discovered() {
    let interpreted = interpret_line(
        CommandStream::Stdout,
        r#"{"type":"session.created","session_id":"abc"}"#,
    );
    assert_eq!(interpreted.native_session_id.as_deref(), Some("abc"));

    // Event names differ between Codex builds; the documented alternates
    // are all accepted so a rename does not silently lose the ability to
    // resume a session.
    for line in [
        r#"{"type":"thread.started","thread_id":"t1"}"#,
        r#"{"type":"session_configured","conversation_id":"c1"}"#,
    ] {
        assert!(interpret_line(CommandStream::Stdout, line)
            .native_session_id
            .is_some());
    }
}

#[test]
fn agent_messages_commands_and_file_changes_map_to_normalized_events() {
    let message = interpret_line(
        CommandStream::Stdout,
        r#"{"type":"item.completed","item":{"type":"agent_message","text":"done"}}"#,
    );
    assert!(matches!(
        message.events[0],
        ProviderEvent::AssistantTextDelta { .. }
    ));

    let command = interpret_line(
        CommandStream::Stdout,
        r#"{"type":"item.completed","item":{"type":"command_execution","command":"cargo test","exit_code":0}}"#,
    );
    assert!(matches!(
        command.events[0],
        ProviderEvent::CommandStarted { .. }
    ));
    assert!(matches!(
        command.events[1],
        ProviderEvent::CommandCompleted { exit_code: 0 }
    ));

    let file = interpret_line(
        CommandStream::Stdout,
        r#"{"type":"item.completed","item":{"type":"file_change","path":"src/main.rs"}}"#,
    );
    assert!(matches!(file.events[0], ProviderEvent::FileChanged { .. }));
}

#[test]
fn tool_calls_never_persist_their_arguments() {
    let interpreted = interpret_line(
        CommandStream::Stdout,
        r#"{"type":"item.completed","item":{"type":"function_call","name":"write_file","arguments":"{\"path\":\"secret.rs\",\"content\":\"TOKEN=abc\"}"}}"#,
    );
    match &interpreted.events[0] {
        ProviderEvent::ToolRequested { tool_name, summary } => {
            assert_eq!(tool_name, "write_file");
            assert!(!summary.contains("TOKEN"), "{summary}");
            assert!(!summary.contains("secret.rs"), "{summary}");
        }
        other => panic!("expected ToolRequested, got {other:?}"),
    }
}

#[test]
fn turn_completion_reports_exact_token_usage_and_a_turn_without_counts_is_unknown() {
    let with_counts = interpret_line(
        CommandStream::Stdout,
        r#"{"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":5,"cached_input_tokens":2}}"#,
    );
    match with_counts.usage {
        Some(UsageObservation::Exact { detail }) => {
            assert!(
                detail.contains("input=10") && detail.contains("cached_input=2"),
                "{detail}"
            );
        }
        other => panic!("expected exact usage, got {other:?}"),
    }

    // No `usage` object at all: nothing was reported, so nothing is claimed.
    let without_usage = interpret_line(CommandStream::Stdout, r#"{"type":"turn.completed"}"#);
    assert_eq!(without_usage.usage, None);

    // A `usage` object with no token counts present: the provider did report
    // usage, but not numbers NACC can quote, so it must be labelled Unknown
    // rather than zero-filled.
    let without_counts = interpret_line(
        CommandStream::Stdout,
        r#"{"type":"turn.completed","usage":{}}"#,
    );
    assert_eq!(
        without_counts.usage,
        Some(UsageObservation::Unknown),
        "a turn with no reported token counts must not become invented numbers"
    );
}

#[test]
fn a_failed_turn_is_terminal_and_an_approval_request_is_never_auto_approved() {
    let failed = interpret_line(
        CommandStream::Stdout,
        r#"{"type":"turn.failed","message":"sandbox denied write"}"#,
    );
    assert!(matches!(
        failed.events[0],
        ProviderEvent::TerminalError { .. }
    ));

    let approval = interpret_line(
        CommandStream::Stdout,
        r#"{"type":"approval.requested","summary":"write to C:/repo/src/lib.rs"}"#,
    );
    assert!(
        matches!(approval.events[0], ProviderEvent::ApprovalRequested { .. }),
        "an approval request is surfaced, never answered by the adapter"
    );
}

#[test]
fn unknown_event_types_are_ignored_not_guessed() {
    let interpreted = interpret_line(
        CommandStream::Stdout,
        r#"{"type":"totally.new.event","payload":{"x":1}}"#,
    );
    assert!(interpreted.events.is_empty());
    let unknown_item = interpret_line(
        CommandStream::Stdout,
        r#"{"type":"item.completed","item":{"type":"brand_new_item"}}"#,
    );
    assert!(unknown_item.events.is_empty());
}

// --- probing / capabilities / contract ----------------------------------

#[tokio::test]
async fn installation_probe_reports_the_version_verbatim() {
    let provider = CodexProvider::new(version_runner());
    let probe = provider
        .probe_installation(&RuntimeProfile {
            location: nacc_provider_core::RuntimeLocation::NativeWindows,
            working_directory: ".".into(),
        })
        .await
        .unwrap();
    assert!(probe.installed);
    assert_eq!(probe.version.as_deref(), Some("codex-cli 0.149.1"));
}

/// A runner whose executable cannot start: the not-installed path.
struct MissingExecutableRunner;

#[async_trait]
impl CommandRunner for MissingExecutableRunner {
    async fn run(
        &self,
        program: &str,
        _args: &[String],
        _cwd: &Path,
        _env: &[(String, String)],
    ) -> Result<CommandOutput> {
        Err(ProviderError::NotInstalled {
            detail: format!("{program} was not found on PATH"),
        })
    }

    async fn spawn(
        &self,
        program: &str,
        _args: &[String],
        _cwd: &Path,
        _env: &[(String, String)],
        _sink: Arc<dyn CommandLineSink>,
    ) -> Result<Arc<dyn nacc_provider_core::RunningCommand>> {
        Err(ProviderError::NotInstalled {
            detail: format!("{program} was not found on PATH"),
        })
    }
}

#[tokio::test]
async fn a_missing_cli_is_an_honest_not_installed_result() {
    let provider = CodexProvider::new(Arc::new(MissingExecutableRunner));
    let probe = provider
        .probe_installation(&RuntimeProfile {
            location: nacc_provider_core::RuntimeLocation::NativeWindows,
            working_directory: ".".into(),
        })
        .await
        .unwrap();
    assert!(!probe.installed);
    assert!(probe.version.is_none());
}

#[tokio::test]
async fn capabilities_report_that_codex_cancellation_is_not_documented() {
    let provider = CodexProvider::new(version_runner());
    let context = CapabilityContext {
        account: AccountProfile {
            id: nacc_domain::ProviderAccountId::new(),
            provider: ProviderId::Codex,
            label: "test".into(),
        },
        runtime: RuntimeProfile {
            location: nacc_provider_core::RuntimeLocation::NativeWindows,
            working_directory: ".".into(),
        },
    };
    let snapshot = provider.capabilities(&context).await.unwrap();
    assert_eq!(snapshot.provider, ProviderId::Codex);
    assert!(
        !snapshot.cancellation_documented,
        "Phase 0 found no cancellation contract for Codex; reporting true would be a guess"
    );
    assert_eq!(
        snapshot.acp_transport,
        nacc_provider_core::AcpTransport::Unsupported
    );
    assert_eq!(
        snapshot.health,
        ProviderHealth::from_probes(&snapshot.installation, &snapshot.auth, None)
    );
}

#[tokio::test]
async fn validate_profile_refuses_unreported_models_and_flags_clamped_levels() {
    let provider = CodexProvider::new(version_runner());

    let unknown = provider
        .validate_profile(&profile(
            "gpt-5",
            ReasoningLevel::High,
            PermissionProfile::ReadOnly,
        ))
        .await
        .unwrap();
    assert!(!unknown.supported);

    let clamped = provider
        .validate_profile(&profile(
            "gpt-5-codex",
            ReasoningLevel::Maximum,
            PermissionProfile::ReadOnly,
        ))
        .await
        .unwrap();
    assert!(clamped.supported);
    assert!(clamped.issues.iter().any(|issue| issue.contains("clamped")));

    let exact = provider
        .validate_profile(&profile(
            "gpt-5-codex",
            ReasoningLevel::High,
            PermissionProfile::ReadOnly,
        ))
        .await
        .unwrap();
    assert!(exact.supported);
    assert!(exact.issues.is_empty(), "{:?}", exact.issues);
}

// --- launch --------------------------------------------------------------

#[tokio::test]
async fn launch_streams_events_in_order_and_records_the_provider_session_id() {
    let provider = CodexProvider::new(jsonl_runner());
    let sink = Arc::new(RecordingSink::new());
    let handle = provider
        .launch(
            LaunchRequest {
                profile: profile(
                    "gpt-5-codex",
                    ReasoningLevel::Auto,
                    PermissionProfile::ReadOnly,
                ),
                working_directory: std::env::temp_dir().to_string_lossy().into_owned(),
                prompt: "contract suite probe".to_string(),
            },
            Box::new(SharingSink(Arc::clone(&sink))),
        )
        .await
        .unwrap();

    wait_until_finished(&provider, &handle.session_id).await;

    let events = sink.events();
    assert!(matches!(
        events.first(),
        Some(ProviderEvent::SessionStarted { .. })
    ));
    assert!(events
        .iter()
        .any(|event| matches!(event, ProviderEvent::CommandStarted { .. })));
    assert!(events
        .iter()
        .any(|event| matches!(event, ProviderEvent::FileChanged { .. })));
    assert!(matches!(
        events.last(),
        Some(ProviderEvent::SessionCompleted)
    ));

    // The provider's own id is what `resume` needs, and it must be recorded
    // even though NACC generated a different id for this launch.
    let entry = provider
        .sessions()
        .get(&handle.session_id)
        .expect("a finished session is retained so its usage and native id remain readable");
    assert_eq!(
        entry.state().native_session_id().as_deref(),
        Some("0192f0aa-1111-7222-8333-444455556666")
    );

    match provider.collect_usage(&handle.session_id).await.unwrap() {
        Some(UsageObservation::Exact { detail }) => {
            assert!(detail.contains("input=2048"), "{detail}")
        }
        other => panic!("expected exact usage, got {other:?}"),
    }
}

#[tokio::test]
async fn cancelling_a_finished_session_is_an_error_rather_than_a_false_success() {
    let provider = CodexProvider::new(jsonl_runner());
    let handle = provider
        .launch(
            LaunchRequest {
                profile: profile(
                    "gpt-5-codex",
                    ReasoningLevel::Auto,
                    PermissionProfile::ReadOnly,
                ),
                working_directory: std::env::temp_dir().to_string_lossy().into_owned(),
                prompt: "contract suite probe".to_string(),
            },
            Box::new(SharingSink(Arc::new(RecordingSink::new()))),
        )
        .await
        .unwrap();
    wait_until_finished(&provider, &handle.session_id).await;

    let err = provider
        .cancel(&handle.session_id, CancellationMode::Forced)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("already finished"),
        "a completed run must not report a successful cancellation: {err}"
    );

    let unknown = provider
        .cancel(&session(), CancellationMode::Forced)
        .await
        .unwrap_err();
    assert!(unknown.to_string().contains("no live session"));
}

#[tokio::test]
async fn the_adapter_passes_its_own_contract_suite() {
    let provider = CodexProvider::new(jsonl_runner());
    let harness = ContractHarness::new(ProviderId::Codex, jsonl_runner());
    let report = run_contract_suite(&provider, &harness).await;
    assert!(report.passed(), "{}", report.summary());
}

#[tokio::test]
async fn the_built_command_line_matches_the_documented_shape() {
    let runner = jsonl_runner();
    let provider = CodexProvider::new(Arc::clone(&runner) as Arc<dyn CommandRunner>);
    provider
        .launch(
            LaunchRequest {
                profile: profile(
                    "gpt-5-codex",
                    ReasoningLevel::Auto,
                    PermissionProfile::ReadOnly,
                ),
                working_directory: std::env::temp_dir().to_string_lossy().into_owned(),
                prompt: "contract suite probe".to_string(),
            },
            Box::new(SharingSink(Arc::new(RecordingSink::new()))),
        )
        .await
        .unwrap();

    let observed = runner.observed_commands();
    assert_eq!(observed.len(), 1, "{observed:?}");
    assert_eq!(
        observed[0],
        "codex exec --json -m gpt-5-codex -s read-only -a never contract suite probe"
    );
}

async fn wait_until_finished(provider: &CodexProvider, session: &SessionId) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if provider.sessions().running_count() == 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("session {} never finished", session.0);
}

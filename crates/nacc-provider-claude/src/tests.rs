//! Claude Code adapter tests: pure mappings (exact, no I/O), stream-json
//! interpretation against fixtures, the adapter contract suite, and
//! end-to-end launch/cancel/usage through `FixtureCommandRunner` -- so the
//! *real* adapter code runs in CI even where no `claude` binary exists.
//!
//! Fixture provenance is stated in `fixtures/README.md`: the `--version`
//! output is captured from the installed CLI documented in
//! `docs/provider-adapters/claude-code.md`; the stream-json lines are
//! *documented-shape* samples, not captured output, because Phase 0 never
//! ran a live session. No test here claims the live stream shape was
//! verified -- running the adapter against the real CLI is what would make
//! that claim true.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use nacc_provider_core::{
    run_contract_suite, CommandLine, CommandLineSink, ContractHarness, FixtureCommandRunner,
    RecordedInvocation, RecordingSink,
};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn load(name: &str) -> Vec<RecordedInvocation> {
    serde_json::from_slice(&std::fs::read(fixture_dir().join(name)).unwrap())
        .unwrap_or_else(|err| panic!("fixture {name} must be valid JSON: {err}"))
}

fn version_runner() -> Arc<FixtureCommandRunner> {
    Arc::new(FixtureCommandRunner::new(load("invocations.json")))
}

fn stream_runner() -> Arc<FixtureCommandRunner> {
    Arc::new(FixtureCommandRunner::new(load("stream-json-run.json")))
}

fn profile(
    model: &str,
    reasoning: ReasoningLevel,
    permission: PermissionProfile,
) -> ResolvedAgentProfile {
    ResolvedAgentProfile {
        account: AccountProfile {
            id: nacc_domain::ProviderAccountId::new(),
            provider: ProviderId::Claude,
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

// --- pure mappings -------------------------------------------------------

#[test]
fn every_canonical_reasoning_level_has_an_explicit_mapping() {
    for (level, expected) in [
        (ReasoningLevel::Auto, (None, true)),
        (ReasoningLevel::Off, (Some("low"), false)),
        (ReasoningLevel::Minimal, (Some("low"), false)),
        (ReasoningLevel::Low, (Some("low"), true)),
        (ReasoningLevel::Medium, (Some("medium"), true)),
        (ReasoningLevel::High, (Some("high"), true)),
        (ReasoningLevel::ExtraHigh, (Some("xhigh"), true)),
        (ReasoningLevel::Maximum, (Some("xhigh"), false)),
    ] {
        assert_eq!(map_reasoning(level), expected, "mapping for {level}");
    }
}

#[test]
fn an_inexact_reasoning_mapping_says_so_in_its_description() {
    // Master plan S10.1: a clamp must be visible. The description is what
    // the GUI shows, so it has to carry the fact rather than the number.
    assert!(reasoning_mapping_description(ReasoningLevel::Maximum).contains("clamped"));
    assert!(reasoning_mapping_description(ReasoningLevel::Off).contains("clamped"));
    assert_eq!(
        reasoning_mapping_description(ReasoningLevel::High),
        "high -> high"
    );
    assert!(reasoning_mapping_description(ReasoningLevel::Auto).contains("CLI default"));
}

#[test]
fn permission_profiles_map_to_documented_modes_and_nothing_sneaks_bypass() {
    assert_eq!(permission_mode(PermissionProfile::ReadOnly), "plan");
    assert_eq!(permission_mode(PermissionProfile::PlanOnly), "plan");
    assert_eq!(
        permission_mode(PermissionProfile::AutonomousWorktree),
        "auto"
    );
    assert_eq!(
        permission_mode(PermissionProfile::RepositoryMaintainer),
        "acceptEdits"
    );
    assert_eq!(
        permission_mode(PermissionProfile::ReleaseCandidate),
        "manual"
    );
    // Only the explicitly dangerous, time-limited profile may reach
    // bypassPermissions (master plan S12.1).
    for profile in [
        PermissionProfile::ReadOnly,
        PermissionProfile::PlanOnly,
        PermissionProfile::AutonomousWorktree,
        PermissionProfile::RepositoryMaintainer,
        PermissionProfile::CiMaintainer,
        PermissionProfile::ReleaseCandidate,
    ] {
        assert_ne!(
            permission_mode(profile),
            "bypassPermissions",
            "{profile:?} must not pre-authorize bypassPermissions"
        );
    }
    assert_eq!(
        permission_mode(PermissionProfile::TemporaryDangerFullAccess),
        "bypassPermissions"
    );
}

#[test]
fn launch_args_are_exactly_what_the_documented_cli_accepts() {
    let id = session();
    let args = build_launch_args(
        &profile(
            "claude-fable-5",
            ReasoningLevel::High,
            PermissionProfile::ReadOnly,
        ),
        "fix the bug",
        &id,
    );
    assert_eq!(
        args,
        vec![
            "--print",
            "fix the bug",
            "--output-format",
            "stream-json",
            "--verbose",
            "--model",
            "claude-fable-5",
            "--permission-mode",
            "plan",
            "--session-id",
            &id.0,
            "--effort",
            "high",
        ]
    );
}

#[test]
fn auto_reasoning_omits_the_effort_flag_rather_than_guessing_a_level() {
    let args = build_launch_args(
        &profile("fable", ReasoningLevel::Auto, PermissionProfile::ReadOnly),
        "p",
        &session(),
    );
    assert!(
        !args.contains(&"--effort".to_string()),
        "Auto must let the CLI decide: {args:?}"
    );
}

#[test]
fn resume_args_use_the_providers_own_session_id() {
    let args = build_resume_args(
        &profile("opus", ReasoningLevel::Medium, PermissionProfile::ReadOnly),
        "11111111-2222-3333-4444-555555555555",
    );
    assert!(args.contains(&"--resume".to_string()));
    assert!(args.contains(&"11111111-2222-3333-4444-555555555555".to_string()));
    assert!(args.contains(&"opus".to_string()));
}

// --- stream-json interpretation -----------------------------------------

#[test]
fn an_init_event_reveals_the_native_session_id() {
    let line =
        r#"{"type":"system","subtype":"init","session_id":"abc-123","model":"claude-fable-5"}"#;
    let interpreted = interpret_line(CommandStream::Stdout, line);
    assert_eq!(interpreted.native_session_id.as_deref(), Some("abc-123"));
    assert!(
        matches!(
            interpreted.usage,
            Some(UsageObservation::Estimated { .. })
        ),
        "the model a session actually resolved to is an estimate of context, never presented as exact"
    );
}

#[test]
fn assistant_text_and_tools_become_normalized_events_without_leaking_inputs() {
    let text = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hello"},{"type":"tool_use","name":"Edit","input":{"file_path":"C:/secret/path.rs","content":"API_KEY=abc"}}]}}"#;
    let interpreted = interpret_line(CommandStream::Stdout, text);
    assert!(matches!(
        interpreted.events[0],
        ProviderEvent::AssistantTextDelta { .. }
    ));
    match &interpreted.events[1] {
        ProviderEvent::ToolRequested { tool_name, summary } => {
            assert_eq!(tool_name, "Edit");
            assert!(summary.contains("file_path"), "{summary}");
            assert!(
                !summary.contains("C:/secret/path.rs") && !summary.contains("API_KEY"),
                "tool inputs routinely hold file contents and secrets; only their shape may \
                 reach the event stream: {summary}"
            );
        }
        other => panic!("expected ToolRequested, got {other:?}"),
    }
}

#[test]
fn a_result_event_reports_exact_usage_when_the_cli_reports_it() {
    let line = r#"{"type":"result","subtype":"success","is_error":false,"result":"plan ready","total_cost_usd":0.5,"usage":{"input_tokens":10,"output_tokens":4}}"#;
    let interpreted = interpret_line(CommandStream::Stdout, line);
    assert!(matches!(
        interpreted.events[0],
        ProviderEvent::PlanArtifactEmitted { .. }
    ));
    match interpreted.usage {
        Some(UsageObservation::Exact { detail }) => {
            assert!(detail.contains("cost_usd=0.5000"), "{detail}");
            assert!(detail.contains("input=10"), "{detail}");
        }
        other => panic!("provider-reported cost must be labelled Exact, got {other:?}"),
    }
}

#[test]
fn an_error_result_is_a_terminal_error_not_a_silent_success() {
    let line =
        r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"boom"}"#;
    let interpreted = interpret_line(CommandStream::Stdout, line);
    assert!(matches!(
        interpreted.events[0],
        ProviderEvent::TerminalError { .. }
    ));
    assert!(interpreted.usage.is_none());
}

#[test]
fn unrecognized_event_types_are_ignored_rather_than_forced_into_the_vocabulary() {
    let interpreted = interpret_line(CommandStream::Stdout, r#"{"type":"brand_new_thing","x":1}"#);
    assert!(interpreted.events.is_empty());
    assert!(interpreted.usage.is_none());
}

#[test]
fn stderr_text_surfaces_as_a_warning_and_bad_stdout_is_not_invented_into_events() {
    let on_stderr = interpret_line(CommandStream::Stderr, "warning: config file ignored");
    assert!(matches!(on_stderr.events[0], ProviderEvent::Warning { .. }));

    let on_stdout = interpret_line(CommandStream::Stdout, "not json at all");
    assert!(
        on_stdout.events.is_empty(),
        "an unparseable stdout line must not be reinterpreted as provider output"
    );
}

#[test]
fn blank_lines_produce_nothing() {
    assert!(interpret_line(CommandStream::Stdout, "   ")
        .events
        .is_empty());
}

// --- probing -------------------------------------------------------------

#[tokio::test]
async fn installation_probe_reports_the_captured_version_string_verbatim() {
    let provider = ClaudeCodeProvider::new(version_runner());
    let probe = provider
        .probe_installation(&RuntimeProfile {
            location: nacc_provider_core::RuntimeLocation::NativeWindows,
            working_directory: ".".into(),
        })
        .await
        .unwrap();
    assert!(probe.installed);
    assert_eq!(probe.version.as_deref(), Some("2.1.215 (Claude Code)"));
    assert_eq!(probe.executable_path.as_deref(), Some("claude"));
}

/// A runner whose executable cannot be started at all: the not-installed
/// path, which must be a normal result rather than an error.
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
async fn a_missing_cli_is_reported_as_not_installed_not_as_a_failure() {
    let provider = ClaudeCodeProvider::new(Arc::new(MissingExecutableRunner));
    let probe = provider
        .probe_installation(&RuntimeProfile {
            location: nacc_provider_core::RuntimeLocation::NativeWindows,
            working_directory: ".".into(),
        })
        .await
        .unwrap();
    assert!(!probe.installed);
    assert!(probe.executable_path.is_none());
    assert!(probe.version.is_none());
}

#[tokio::test]
async fn capabilities_are_self_consistent_and_never_claim_an_acp_transport() {
    let provider = ClaudeCodeProvider::new(version_runner());
    let context = CapabilityContext {
        account: AccountProfile {
            id: nacc_domain::ProviderAccountId::new(),
            provider: ProviderId::Claude,
            label: "test".into(),
        },
        runtime: RuntimeProfile {
            location: nacc_provider_core::RuntimeLocation::NativeWindows,
            working_directory: ".".into(),
        },
    };
    let snapshot = provider.capabilities(&context).await.unwrap();
    assert_eq!(snapshot.provider, ProviderId::Claude);
    // Health must follow from the snapshot's own probes; asserting a fixed
    // value here would make the test depend on whether the machine running
    // it happens to have signed in.
    assert_eq!(
        snapshot.health,
        ProviderHealth::from_probes(&snapshot.installation, &snapshot.auth, None)
    );
    assert!(!snapshot.models.is_empty());
    assert_eq!(
        snapshot.acp_transport,
        nacc_provider_core::AcpTransport::Unsupported,
        "claude itself exposes no ACP surface; only a third-party bridge does, which NACC has not adopted"
    );
}

#[tokio::test]
async fn validate_profile_refuses_an_unreported_model_and_accepts_a_reported_one() {
    let provider = ClaudeCodeProvider::new(version_runner());

    let unknown = provider
        .validate_profile(&profile(
            "some-other-vendors-model",
            ReasoningLevel::High,
            PermissionProfile::ReadOnly,
        ))
        .await
        .unwrap();
    assert!(!unknown.supported);
    assert!(unknown.issues[0].contains("not one this adapter"));

    let known = provider
        .validate_profile(&profile(
            "claude-fable-5",
            ReasoningLevel::High,
            PermissionProfile::ReadOnly,
        ))
        .await
        .unwrap();
    assert!(known.supported);
    assert!(known.issues.is_empty(), "{:?}", known.issues);

    // A clamped level is supported *and* reported, never silent.
    let clamped = provider
        .validate_profile(&profile(
            "fable",
            ReasoningLevel::Maximum,
            PermissionProfile::ReadOnly,
        ))
        .await
        .unwrap();
    assert!(clamped.supported);
    assert!(clamped.issues.iter().any(|issue| issue.contains("inexact")));
}

// --- launch / cancel / usage --------------------------------------------

#[tokio::test]
async fn launch_streams_normalized_events_in_order_and_reports_exact_usage() {
    let provider = ClaudeCodeProvider::new(stream_runner());
    let sink = Arc::new(RecordingSink::new());

    let handle = provider
        .launch(
            LaunchRequest {
                profile: profile("fable", ReasoningLevel::Auto, PermissionProfile::ReadOnly),
                working_directory: std::env::temp_dir().to_string_lossy().into_owned(),
                prompt: "contract suite probe".to_string(),
            },
            Box::new(SharingSink(Arc::clone(&sink))),
        )
        .await
        .expect("a fixture-backed launch must succeed");

    wait_until_finished(&provider, &handle.session_id).await;

    let events = sink.events();
    assert!(
        matches!(events.first(), Some(ProviderEvent::SessionStarted { .. })),
        "SessionStarted must be first: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ProviderEvent::AssistantTextDelta { .. })),
        "assistant text must reach the sink: {events:?}"
    );
    assert!(matches!(
        events.last(),
        Some(ProviderEvent::SessionCompleted)
    ));

    match provider.collect_usage(&handle.session_id).await.unwrap() {
        Some(UsageObservation::Exact { detail }) => {
            assert!(detail.contains("cost_usd=0.0123"), "{detail}")
        }
        other => panic!("expected exact usage after completion, got {other:?}"),
    }
    assert_eq!(
        provider.sessions().running_count(),
        0,
        "a finished session must not still count as running"
    );
}

#[tokio::test]
async fn a_nonzero_exit_with_an_error_result_is_reported_as_a_terminal_error() {
    // The second fixture is a failed run: same argv, error result, exit 1.
    // Only the *first* matching fixture is used per run, so this drives the
    // interpreter directly through the same code path the adapter uses.
    let interpreted = interpret_line(
        CommandStream::Stdout,
        r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"rate limit exceeded, retry after 30s"}"#,
    );
    match &interpreted.events[0] {
        ProviderEvent::TerminalError { message } => assert!(message.contains("rate limit")),
        other => panic!("expected TerminalError, got {other:?}"),
    }
}

#[tokio::test]
async fn cancelling_an_unknown_session_is_a_typed_error() {
    let provider = ClaudeCodeProvider::new(stream_runner());
    let err = provider
        .cancel(&session(), CancellationMode::Forced)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no live session"));
}

#[tokio::test]
async fn launch_refuses_an_unsupported_model_before_running_anything() {
    let provider = ClaudeCodeProvider::new(stream_runner());
    let err = provider
        .launch(
            LaunchRequest {
                profile: profile(
                    "not-a-real-model",
                    ReasoningLevel::High,
                    PermissionProfile::ReadOnly,
                ),
                working_directory: std::env::temp_dir().to_string_lossy().into_owned(),
                prompt: "nope".to_string(),
            },
            Box::new(SharingSink(Arc::new(RecordingSink::new()))),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::UnsupportedSetting { .. }));
}

#[tokio::test]
async fn launch_refuses_a_working_directory_that_does_not_exist() {
    let provider = ClaudeCodeProvider::new(stream_runner());
    let err = provider
        .launch(
            LaunchRequest {
                profile: profile("fable", ReasoningLevel::Auto, PermissionProfile::ReadOnly),
                working_directory: std::env::temp_dir()
                    .join("nacc-claude-no-such-dir")
                    .to_string_lossy()
                    .into_owned(),
                prompt: "p".to_string(),
            },
            Box::new(SharingSink(Arc::new(RecordingSink::new()))),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("working directory does not exist"));
}

#[tokio::test]
async fn send_input_on_a_one_shot_run_is_reported_as_unsupported() {
    let provider = ClaudeCodeProvider::new(stream_runner());
    let handle = provider
        .launch(
            LaunchRequest {
                profile: profile("fable", ReasoningLevel::Auto, PermissionProfile::ReadOnly),
                working_directory: std::env::temp_dir().to_string_lossy().into_owned(),
                prompt: "contract suite probe".to_string(),
            },
            Box::new(SharingSink(Arc::new(RecordingSink::new()))),
        )
        .await
        .unwrap();
    let err = provider
        .send_input(
            &handle.session_id,
            nacc_provider_core::AgentInput::Text {
                text: "more".into(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::UnsupportedSetting { .. }));
}

#[tokio::test]
async fn the_adapter_passes_its_own_contract_suite() {
    // The suite drives the real adapter through the fixture runner,
    // including a launch whose argv contains a fresh session UUID -- which
    // is why the fixture's args carry a `*` wildcard.
    let provider = ClaudeCodeProvider::new(stream_runner());
    let harness = ContractHarness::new(ProviderId::Claude, stream_runner());
    let report = run_contract_suite(&provider, &harness).await;
    assert!(report.passed(), "{}", report.summary());
}

/// `EventSink` is `Send + Sync` and boxed; the suite and the tests want to
/// keep a handle to inspect afterwards.
struct SharingSink(Arc<RecordingSink>);

impl EventSink for SharingSink {
    fn emit(&self, event: ProviderEvent) {
        self.0.emit(event);
    }
}

async fn wait_until_finished(provider: &ClaudeCodeProvider, session: &SessionId) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if provider.sessions().running_count() == 0 {
            // Give the terminal event a moment to land in the sink after
            // the process bookkeeping completes.
            tokio::time::sleep(Duration::from_millis(50)).await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("session {} never finished", session.0);
}

#[tokio::test]
async fn the_fixture_runner_reports_the_command_lines_the_adapter_built() {
    let runner = stream_runner();
    let provider = ClaudeCodeProvider::new(Arc::clone(&runner) as Arc<dyn CommandRunner>);
    provider
        .launch(
            LaunchRequest {
                profile: profile("fable", ReasoningLevel::Auto, PermissionProfile::ReadOnly),
                working_directory: std::env::temp_dir().to_string_lossy().into_owned(),
                prompt: "contract suite probe".to_string(),
            },
            Box::new(SharingSink(Arc::new(RecordingSink::new()))),
        )
        .await
        .unwrap();

    let observed = runner.observed_commands();
    assert_eq!(observed.len(), 1, "{observed:?}");
    assert!(observed[0].starts_with("claude --print contract suite probe"));
    assert!(observed[0].contains("--output-format stream-json"));
    assert!(observed[0].contains("--permission-mode plan"));
}

/// Marker so the unused-import lint does not fire for `CommandLine`, which
/// is part of the sink trait's signature this module implements.
#[allow(dead_code)]
fn _sink_signature_shape(line: CommandLine) -> String {
    line.text
}

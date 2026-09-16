//! Provider adapter for the OpenAI Codex CLI (master plan S9.2) -- Phase
//! 5's second half.
//!
//! Built against `docs/provider-adapters/codex.md`, which records the CLI
//! surface verified live on this machine (`codex-cli 0.149.1`). That
//! document is unusually specific about three things this adapter has to get
//! right, and each is handled explicitly rather than glossed:
//!
//! - **`--full-auto` no longer exists.** It does not appear anywhere in the
//!   installed help output; `--sandbox`/`--ask-for-approval` are the real
//!   controls. Any reference to `--full-auto` in older material is stale.
//! - **Reasoning effort has no flag.** It is a config override
//!   (`-c model_reasoning_effort=…`), and its vocabulary is
//!   `minimal|low|medium|high|xhigh` -- no `none`, no `max`. Canonical `Off`
//!   collapses to `minimal` and `Maximum` clamps to `xhigh`; both are
//!   reported as inexact so the GUI shows the effective level (master plan
//!   S10.1: never silently downgrade).
//! - **`--dangerously-bypass-approvals-and-sandbox` is never used by this
//!   adapter.** The Phase 0 contract doc records a real open issue where
//!   that mode can stall forever awaiting a forced approval, and master plan
//!   S12.2 requires dangerous operations to be approval-gated. Even the
//!   temporary dangerous permission profile maps to the *sandboxed*
//!   `danger-full-access` sandbox plus `on-request` approval -- never to the
//!   bypass flag.
//!
//! Session resume is `codex exec resume <SESSION_ID>`; authentication is
//! detected by the presence of `~/.codex/auth.json`, whose contents are
//! never read (master plan S8.4).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use nacc_domain::{ModelId, PermissionProfile, ProviderId, ReasoningLevel, ThinkingMode};
use nacc_provider_core::{
    launch_streaming_session, AccountProfile, AgentInput, AgentProvider, AgentSessionHandle,
    AuthProbe, CancellationMode, CapabilityContext, CapabilitySnapshot, CommandOutput,
    CommandRunner, CommandStream, EventSink, InstallationProbe, LaunchRequest, LineInterpretation,
    ModelDescriptor, ProfileValidation, ProviderError, ProviderEvent, ProviderHealth,
    ResolvedAgentProfile, Result, ResumeRequest, RuntimeProfile, SessionId, SessionSupervisor,
    StreamingLaunch, UsageObservation,
};

pub const DEFAULT_EXECUTABLE: &str = "codex";

/// Sandbox mode for a permission profile (documented values: `read-only`,
/// `workspace-write`, `danger-full-access`).
pub fn sandbox_mode(profile: PermissionProfile) -> &'static str {
    match profile {
        PermissionProfile::ReadOnly | PermissionProfile::PlanOnly => "read-only",
        PermissionProfile::AutonomousWorktree
        | PermissionProfile::RepositoryMaintainer
        | PermissionProfile::CiMaintainer
        | PermissionProfile::ReleaseCandidate => "workspace-write",
        PermissionProfile::TemporaryDangerFullAccess => "danger-full-access",
    }
}

/// Approval policy (documented enumerations observed on the installed CLI:
/// `on-request`, `never`). `never` is used only for the non-mutating
/// profiles, where there is nothing to approve.
pub fn approval_policy(profile: PermissionProfile) -> &'static str {
    match profile {
        PermissionProfile::ReadOnly | PermissionProfile::PlanOnly => "never",
        _ => "on-request",
    }
}

/// Canonical reasoning level -> Codex's config-override value, with whether
/// the mapping is exact (see the module doc).
pub fn map_reasoning(level: ReasoningLevel) -> (Option<&'static str>, bool) {
    match level {
        ReasoningLevel::Auto => (None, true),
        // Codex's floor is `minimal`; there is no "none".
        ReasoningLevel::Off | ReasoningLevel::Minimal => (Some("minimal"), false),
        ReasoningLevel::Low => (Some("low"), true),
        ReasoningLevel::Medium => (Some("medium"), true),
        ReasoningLevel::High => (Some("high"), true),
        ReasoningLevel::ExtraHigh => (Some("xhigh"), true),
        // Nothing above `xhigh` exists, so `Maximum` clamps.
        ReasoningLevel::Maximum => (Some("xhigh"), false),
    }
}

/// Codex exposes no separate thinking control; the model handles it.
pub const THINKING: ThinkingMode = ThinkingMode::Unsupported;

/// Build the argv for a non-interactive run: `codex exec [OPTIONS] <PROMPT>`.
pub fn build_launch_args(profile: &ResolvedAgentProfile, prompt: &str) -> Vec<String> {
    let mut args = vec![
        "exec".to_string(),
        "--json".to_string(),
        "-m".to_string(),
        profile.model.0.clone(),
        "-s".to_string(),
        sandbox_mode(profile.permission).to_string(),
        "-a".to_string(),
        approval_policy(profile.permission).to_string(),
    ];
    if let (Some(effort), _) = map_reasoning(profile.reasoning) {
        args.push("-c".to_string());
        args.push(format!("model_reasoning_effort={effort}"));
    }
    args.push(prompt.to_string());
    args
}

/// `codex exec resume <SESSION_ID>` (the documented subcommand form).
pub fn build_resume_args(profile: &ResolvedAgentProfile, native_session_id: &str) -> Vec<String> {
    let mut args = vec![
        "exec".to_string(),
        "--json".to_string(),
        "resume".to_string(),
        native_session_id.to_string(),
    ];
    if !profile.model.0.is_empty() {
        args.push("-m".to_string());
        args.push(profile.model.0.clone());
    }
    if let (Some(effort), _) = map_reasoning(profile.reasoning) {
        args.push("-c".to_string());
        args.push(format!("model_reasoning_effort={effort}"));
    }
    args
}

/// Interpret one JSONL line from `codex exec --json`.
///
/// The documented shape is one JSON object per line with a `type` field. The
/// event vocabulary is Codex's own (it is *not* ACP -- the Phase 0 contract
/// doc grepped the whole help output for ACP and found nothing), and a live
/// stream was never captured in Phase 0, so the fixture is a documented-shape
/// sample (see `fixtures/README.md`). The interpreter therefore maps the
/// event kinds the contract doc names and **ignores** everything else at
/// TRACE level rather than guessing: a provider that adds events must not be
/// able to break a run by telling NACC something new.
pub fn interpret_line(stream: CommandStream, line: &str) -> LineInterpretation {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return LineInterpretation::none();
    }
    let value: Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        Err(err) => {
            if stream == CommandStream::Stderr {
                return LineInterpretation::events(vec![ProviderEvent::Warning {
                    message: trimmed.to_string(),
                }]);
            }
            tracing::trace!(error = %err, line = %trimmed, "unparseable codex jsonl line");
            return LineInterpretation::none();
        }
    };

    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        // Session/thread creation reveals the provider's own id, which
        // `resume` needs.
        "session.created" | "thread.started" | "session.start" | "session_configured" => {
            let mut interpretation = LineInterpretation::none();
            if let Some(id) = value
                .get("session_id")
                .or_else(|| value.get("thread_id"))
                .or_else(|| value.get("conversation_id"))
                .and_then(Value::as_str)
            {
                interpretation = interpretation.with_native_session_id(id);
            }
            interpretation
        }
        "item.started" | "item.completed" | "item.updated" => {
            let Some(item) = value.get("item") else {
                return LineInterpretation::none();
            };
            let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
            match item_type {
                "agent_message" | "assistant_message" => {
                    match item.get("text").and_then(Value::as_str) {
                        Some(text) if !text.is_empty() => {
                            LineInterpretation::events(vec![ProviderEvent::AssistantTextDelta {
                                text: text.to_string(),
                            }])
                        }
                        _ => LineInterpretation::none(),
                    }
                }
                "reasoning" | "reasoning_summary" => {
                    LineInterpretation::events(vec![ProviderEvent::ReasoningStatus {
                        status: "thinking".to_string(),
                    }])
                }
                "command_execution" | "local_shell_call" => {
                    let command = item
                        .get("command")
                        .and_then(Value::as_str)
                        .unwrap_or("(command not reported)");
                    let mut events = vec![ProviderEvent::CommandStarted {
                        command: command.to_string(),
                    }];
                    if let Some(exit) = item.get("exit_code").and_then(Value::as_i64) {
                        events.push(ProviderEvent::CommandCompleted {
                            exit_code: exit as i32,
                        });
                    }
                    LineInterpretation::events(events)
                }
                "file_change" | "patch_apply" => match item.get("path").and_then(Value::as_str) {
                    Some(path) => LineInterpretation::events(vec![ProviderEvent::FileChanged {
                        path: path.to_string(),
                    }]),
                    None => LineInterpretation::none(),
                },
                "mcp_tool_call" | "function_call" | "tool_call" => {
                    let name = item
                        .get("name")
                        .or_else(|| item.get("tool_name"))
                        .and_then(Value::as_str)
                        .unwrap_or("unknown-tool");
                    LineInterpretation::events(vec![ProviderEvent::ToolRequested {
                        tool_name: name.to_string(),
                        summary:
                            "arguments are not persisted (may contain file contents or secrets)"
                                .to_string(),
                    }])
                }
                "error" => LineInterpretation::events(vec![ProviderEvent::RecoverableError {
                    message: item
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("codex reported an item-level error")
                        .to_string(),
                }]),
                _ => {
                    tracing::trace!(item_type, "ignoring unrecognized codex item type");
                    LineInterpretation::none()
                }
            }
        }
        "turn.completed" | "turn.finished" => {
            let usage = value.get("usage").map(usage_from_json);
            let mut interpretation =
                LineInterpretation::events(vec![ProviderEvent::UsageUpdated {
                    detail: "turn completed".to_string(),
                }]);
            interpretation.usage = usage;
            interpretation
        }
        "turn.failed" | "error" | "stream.error" => {
            LineInterpretation::events(vec![ProviderEvent::TerminalError {
                message: value
                    .get("message")
                    .or_else(|| value.get("error"))
                    .and_then(Value::as_str)
                    .unwrap_or("codex reported a failed turn")
                    .to_string(),
            }])
        }
        // Codex documents approval requests as a real interaction; NACC
        // never answers them itself (an approval is a human decision, S12),
        // it surfaces them.
        "approval.requested" | "exec_approval_request" => {
            LineInterpretation::events(vec![ProviderEvent::ApprovalRequested {
                summary: value
                    .get("summary")
                    .or_else(|| value.get("reason"))
                    .and_then(Value::as_str)
                    .unwrap_or("codex requested approval")
                    .to_string(),
            }])
        }
        "rate_limit" | "rate_limited" => {
            LineInterpretation::events(vec![ProviderEvent::RecoverableError {
                message: format!(
                    "rate limited: {}",
                    value
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("no detail reported")
                ),
            }])
        }
        _ => {
            tracing::trace!(event_type, "ignoring unrecognized codex jsonl event type");
            LineInterpretation::none()
        }
    }
}

fn usage_from_json(usage: &Value) -> UsageObservation {
    let mut parts = Vec::new();
    for (label, key) in [
        ("input", "input_tokens"),
        ("output", "output_tokens"),
        ("cached_input", "cached_input_tokens"),
    ] {
        if let Some(count) = usage.get(key).and_then(Value::as_u64) {
            parts.push(format!("{label}={count}"));
        }
    }
    if parts.is_empty() {
        // A turn completed without token counts is still a usage signal,
        // but NACC must not invent numbers for it.
        UsageObservation::Unknown
    } else {
        UsageObservation::Exact {
            detail: format!("tokens {}", parts.join(", ")),
        }
    }
}

/// Models this adapter offers before any live discovery exists.
///
/// The contract doc verified `-m/--model` and the local-provider flags but
/// did not enumerate model ids, so this list is deliberately short and
/// explicitly documented as the documented aliases rather than a scraped
/// catalogue. `list_models` and the capability snapshot both come from here,
/// so they cannot disagree (the contract suite checks exactly that).
pub fn documented_models() -> Vec<ModelDescriptor> {
    [
        ("gpt-5-codex", "gpt-5-codex"),
        ("gpt-5.4", "gpt-5.4"),
        ("o4-mini", "o4-mini"),
    ]
    .into_iter()
    .map(|(id, display_name)| ModelDescriptor {
        id: ModelId(id.to_string()),
        display_name: display_name.to_string(),
        reasoning_levels: vec![
            ReasoningLevel::Auto,
            ReasoningLevel::Minimal,
            ReasoningLevel::Low,
            ReasoningLevel::Medium,
            ReasoningLevel::High,
            ReasoningLevel::ExtraHigh,
        ],
        thinking: THINKING,
        structured_output: true,
        context_window_tokens: None,
    })
    .collect()
}

pub struct CodexProvider {
    runner: Arc<dyn CommandRunner>,
    executable: String,
    sessions: SessionSupervisor,
}

impl CodexProvider {
    pub fn new(runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            runner,
            executable: DEFAULT_EXECUTABLE.to_string(),
            sessions: SessionSupervisor::new(),
        }
    }

    pub fn with_executable(mut self, executable: impl Into<String>) -> Self {
        self.executable = executable.into();
        self
    }

    pub fn executable(&self) -> &str {
        &self.executable
    }

    pub fn sessions(&self) -> &SessionSupervisor {
        &self.sessions
    }

    async fn run_probe(&self, args: &[&str]) -> Result<CommandOutput> {
        self.runner
            .run(
                &self.executable,
                &args.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
                Path::new("."),
                &[],
            )
            .await
    }

    /// `~/.codex/auth.json` -- existence checked, contents never read.
    fn auth_store_path() -> Option<PathBuf> {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(|home| PathBuf::from(home).join(".codex").join("auth.json"))
    }
}

#[async_trait]
impl AgentProvider for CodexProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }

    fn display_name(&self) -> &str {
        "OpenAI Codex CLI"
    }

    async fn probe_installation(&self, _runtime: &RuntimeProfile) -> Result<InstallationProbe> {
        match self.run_probe(&["--version"]).await {
            Ok(output) => Ok(InstallationProbe {
                installed: output.succeeded(),
                executable_path: output.succeeded().then(|| self.executable.clone()),
                version: output
                    .stdout
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .map(str::to_string),
            }),
            Err(ProviderError::NotInstalled { .. }) | Err(ProviderError::Process(_)) => {
                Ok(InstallationProbe {
                    installed: false,
                    executable_path: None,
                    version: None,
                })
            }
            Err(other) => Err(other),
        }
    }

    async fn probe_authentication(&self, _account: &AccountProfile) -> Result<AuthProbe> {
        match Self::auth_store_path() {
            Some(path) if path.is_file() => Ok(AuthProbe {
                authenticated: true,
                account_label: None,
                detail: Some(format!(
                    "auth store present at {} (contents never read)",
                    path.display()
                )),
            }),
            Some(path) => Ok(AuthProbe {
                authenticated: false,
                account_label: None,
                detail: Some(format!(
                    "no auth store at {}; run `codex login` (or sign in interactively once)",
                    path.display()
                )),
            }),
            None => Ok(AuthProbe {
                authenticated: false,
                account_label: None,
                detail: Some(
                    "neither USERPROFILE nor HOME is set, so NACC cannot locate the auth store"
                        .to_string(),
                ),
            }),
        }
    }

    async fn list_models(&self, _account: &AccountProfile) -> Result<Vec<ModelDescriptor>> {
        Ok(documented_models())
    }

    async fn capabilities(&self, context: &CapabilityContext) -> Result<CapabilitySnapshot> {
        let installation = self.probe_installation(&context.runtime).await?;
        let auth = self.probe_authentication(&context.account).await?;
        let health = ProviderHealth::from_probes(&installation, &auth, None);
        Ok(CapabilitySnapshot {
            provider: self.id(),
            runtime: context.runtime.location,
            installation,
            auth,
            health,
            models: documented_models(),
            noninteractive_mode: true,
            structured_json_output: true,
            streaming_json_output: true,
            interactive_pty: false,
            session_resume: true,
            custom_agents: true,
            subagents: false,
            mcp: true,
            // Verified live in Phase 0: no `--acp` flag or `acp` subcommand
            // exists anywhere in the installed CLI, native or bridged.
            acp_transport: nacc_provider_core::AcpTransport::Unsupported,
            usage_reporting: true,
            // Cancellation is explicitly *unverified* for Codex (the
            // contract doc says so), unlike Claude's documented SIGTERM
            // contract. NACC still contains and terminates the tree; what is
            // unverified is the CLI's own graceful behavior, and this flag
            // reports exactly that.
            cancellation_documented: false,
            captured_at_millis: now_millis(),
        })
    }

    async fn validate_profile(&self, profile: &ResolvedAgentProfile) -> Result<ProfileValidation> {
        let models = documented_models();
        let mut issues = Vec::new();
        let known = models.iter().find(|model| model.id == profile.model);
        if known.is_none() {
            issues.push(format!(
                "model {} is not one this adapter reports ({}); refusing to launch with an \
                 unverified model",
                profile.model,
                models
                    .iter()
                    .map(|model| model.id.0.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            return Ok(ProfileValidation {
                supported: false,
                issues,
            });
        }
        if matches!(profile.reasoning, ReasoningLevel::Maximum) {
            issues.push(
                "reasoning level maximum maps onto Codex's xhigh (clamped; the effective level is \
                 shown before the run starts)"
                    .to_string(),
            );
        }
        if matches!(profile.reasoning, ReasoningLevel::Off) {
            issues.push(
                "reasoning level off maps onto Codex's minimal (Codex has no 'none' level)"
                    .to_string(),
            );
        }
        Ok(ProfileValidation {
            supported: true,
            issues,
        })
    }

    async fn launch(
        &self,
        request: LaunchRequest,
        sink: Box<dyn EventSink>,
    ) -> Result<AgentSessionHandle> {
        let validation = self.validate_profile(&request.profile).await?;
        if !validation.supported {
            return Err(ProviderError::UnsupportedSetting {
                detail: validation.issues.join("; "),
            });
        }
        let cwd = PathBuf::from(&request.working_directory);
        if !cwd.is_dir() {
            return Err(ProviderError::Other(format!(
                "working directory does not exist: {}",
                cwd.display()
            )));
        }
        let args = build_launch_args(&request.profile, &request.prompt);
        launch_streaming_session(
            self.runner.as_ref(),
            &self.sessions,
            StreamingLaunch {
                provider: self.id(),
                program: &self.executable,
                args: &args,
                working_directory: &cwd,
                env: &[],
                // Codex reports its own session id in its first event, so
                // NACC's id is generated here and the provider's id is
                // recorded separately for `resume`.
                session_id: SessionId(nacc_domain::WorkflowRunId::new().to_string()),
                model: request.profile.model.clone(),
                interpreter: Arc::new(interpret_line),
            },
            Arc::from(sink),
            None,
        )
        .await
    }

    async fn send_input(&self, session: &SessionId, _input: AgentInput) -> Result<()> {
        if self.sessions.get(session).is_none() {
            return Err(ProviderError::Other(format!(
                "no live session with id {}",
                session.0
            )));
        }
        Err(ProviderError::UnsupportedSetting {
            detail: "codex exec is one-shot from NACC's side; use resume to continue".to_string(),
        })
    }

    async fn cancel(&self, session: &SessionId, mode: CancellationMode) -> Result<()> {
        self.sessions.cancel(session, mode).await
    }

    async fn resume(
        &self,
        request: ResumeRequest,
        sink: Box<dyn EventSink>,
    ) -> Result<AgentSessionHandle> {
        let args = build_resume_args(&request.profile, &request.session_id.0);
        let cwd = PathBuf::from(&request.profile.runtime.working_directory);
        launch_streaming_session(
            self.runner.as_ref(),
            &self.sessions,
            StreamingLaunch {
                provider: self.id(),
                program: &self.executable,
                args: &args,
                working_directory: &cwd,
                env: &[],
                session_id: request.session_id.clone(),
                model: request.profile.model.clone(),
                interpreter: Arc::new(interpret_line),
            },
            Arc::from(sink),
            None,
        )
        .await
    }

    async fn collect_usage(&self, session: &SessionId) -> Result<Option<UsageObservation>> {
        match self.sessions.get(session) {
            Some(entry) => Ok(entry.state().usage()),
            None => Ok(None),
        }
    }
}

pub fn reasoning_mapping_description(level: ReasoningLevel) -> String {
    match map_reasoning(level) {
        (Some(value), true) => format!("{level} -> {value}"),
        (Some(value), false) => format!("{level} -> {value} (clamped/collapsed)"),
        (None, _) => format!("{level} -> CLI default (config override omitted)"),
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;

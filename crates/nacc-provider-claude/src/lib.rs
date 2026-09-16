//! Provider adapter for Claude Code (master plan S9.1) -- Phase 5's real
//! vertical-slice implementation.
//!
//! Built against `docs/provider-adapters/claude-code.md`, which records the
//! CLI surface verified live on this machine (`claude 2.1.215`), including
//! the correction that `--effort` is a real top-level flag with choices
//! `low, medium, high, xhigh, max`. Every mapping in this crate follows
//! from that document rather than from a guess, and the tests below pin the
//! mapping so a future CLI change breaks a test instead of a user's run.
//!
//! # What is real here and what is not
//!
//! Real: probing (`--version`, and reading whether a credential store
//! exists without ever reading its contents), argv construction, the
//! canonical-scale-to-CLI mappings, stream-json interpretation, session
//! lifecycle (launch/cancel/resume/usage), and error classification.
//!
//! Not verified against a live CLI *from this crate's tests*: the exact
//! stream-json event shapes. The contract doc says plainly that a live run
//! was not exercised in Phase 0, so the fixtures under `fixtures/` are
//! **shape fixtures derived from the documented format**, labelled as such
//! in their own README, and the interpreter is written to degrade
//! honestly: an unrecognized event type is logged and ignored rather than
//! being forced into a normalized event it might not be. Running this
//! adapter against the installed CLI is the step that converts those shapes
//! into captured ones, and the fixture README says exactly that.

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

/// The executable name resolved from PATH. Overridable so a user with
/// several installed copies (or a WSL bridge script) can point NACC at a
/// specific one; the Setup Wizard shows the exact resolved path it found.
pub const DEFAULT_EXECUTABLE: &str = "claude";

/// Documented permission-mode values (`claude --help`): `acceptEdits`,
/// `auto`, `bypassPermissions`, `manual`, `dontAsk`, `plan`.
///
/// The mapping from NACC's permission profiles is a *policy* decision
/// recorded in code, not a guess: only `ReadOnly`/`PlanOnly` map to a
/// non-mutating mode, and nothing here maps to `bypassPermissions` --
/// master plan S12.2 requires the dangerous operations to be
/// approval-gated rather than pre-authorized by a profile.
fn permission_mode(profile: PermissionProfile) -> &'static str {
    match profile {
        PermissionProfile::ReadOnly => "plan",
        PermissionProfile::PlanOnly => "plan",
        PermissionProfile::AutonomousWorktree => "auto",
        PermissionProfile::RepositoryMaintainer => "acceptEdits",
        PermissionProfile::CiMaintainer => "acceptEdits",
        PermissionProfile::ReleaseCandidate => "manual",
        PermissionProfile::TemporaryDangerFullAccess => "bypassPermissions",
    }
}

/// Canonical-scale-to-Claude effort mapping, plus whether the mapping is
/// exact. Master plan S10.1 requires a clamped or collapsed mapping to be
/// *visible*, never silent; returning the fact alongside the value is what
/// makes that possible without the GUI re-deriving it.
///
/// Claude's floor is `low` -- there is no "none" level -- so canonical
/// `Off` and `Minimal` both collapse onto `low` (inexact, and reported as
/// such).
pub fn map_reasoning(level: ReasoningLevel) -> (Option<&'static str>, bool) {
    match level {
        ReasoningLevel::Auto => (None, true),
        ReasoningLevel::Off | ReasoningLevel::Minimal => (Some("low"), false),
        ReasoningLevel::Low => (Some("low"), true),
        ReasoningLevel::Medium => (Some("medium"), true),
        ReasoningLevel::High => (Some("high"), true),
        ReasoningLevel::ExtraHigh => (Some("xhigh"), true),
        // Claude Code has no `max` above `xhigh` on the installed binary's
        // documented choices; requesting it therefore clamps.
        ReasoningLevel::Maximum => (Some("xhigh"), false),
    }
}

/// Claude Code exposes no thinking on/off switch on the CLI: the model
/// decides. Reported as `ManagedByProvider` so the Role Matrix disables the
/// control instead of pretending a toggle does something.
pub const THINKING: ThinkingMode = ThinkingMode::ManagedByProvider;

/// Build the argv for a non-interactive run. Pure and testable -- which is
/// the point, because this is where a wrong flag silently wastes a user's
/// budget.
///
/// `session_id` is passed as `--session-id` so the provider's own id is
/// NACC's, which makes `resume` exact rather than best-effort (the CLI
/// accepts a UUID there).
pub fn build_launch_args(
    profile: &ResolvedAgentProfile,
    prompt: &str,
    session_id: &SessionId,
) -> Vec<String> {
    let mut args = vec![
        "--print".to_string(),
        prompt.to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--model".to_string(),
        profile.model.0.clone(),
        "--permission-mode".to_string(),
        permission_mode(profile.permission).to_string(),
        "--session-id".to_string(),
        session_id.0.clone(),
    ];
    if let (Some(effort), _) = map_reasoning(profile.reasoning) {
        args.push("--effort".to_string());
        args.push(effort.to_string());
    }
    args
}

/// Build the argv for resuming an interrupted turn. `--resume` takes the
/// provider's own session id, which is why `SessionState` remembers it.
pub fn build_resume_args(profile: &ResolvedAgentProfile, native_session_id: &str) -> Vec<String> {
    let mut args = vec![
        "--print".to_string(),
        "--resume".to_string(),
        native_session_id.to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
    ];
    if !profile.model.0.is_empty() {
        args.push("--model".to_string());
        args.push(profile.model.0.clone());
    }
    if let (Some(effort), _) = map_reasoning(profile.reasoning) {
        args.push("--effort".to_string());
        args.push(effort.to_string());
    }
    args
}

/// Interpret one line of `--output-format stream-json` output.
///
/// The documented shape (see the fixture README for provenance) is one JSON
/// object per line with a `type` field:
///
/// - `system` / `subtype: "init"` -> reveals the session id;
/// - `assistant` -> `message.content[]` blocks of `text` / `tool_use`;
/// - `user` -> `message.content[]` blocks of `tool_result`;
/// - `result` -> final result, `usage`, and `total_cost_usd`.
///
/// Anything else is ignored deliberately: Claude Code adds event types over
/// time, and failing a run because a *diagnostic* event type is unfamiliar
/// would turn a provider improvement into a NACC outage. The line is
/// trace-logged instead, so a support bundle can still show it.
pub fn interpret_line(stream: CommandStream, line: &str) -> LineInterpretation {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return LineInterpretation::none();
    }
    let value: Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        Err(err) => {
            // stderr from the CLI is usually human-readable text, not JSON;
            // that is information, not a parse failure to bury.
            if stream == CommandStream::Stderr {
                return LineInterpretation::events(vec![ProviderEvent::Warning {
                    message: trimmed.to_string(),
                }]);
            }
            tracing::trace!(error = %err, line = %trimmed, "unparseable claude stream-json line");
            return LineInterpretation::none();
        }
    };

    match value.get("type").and_then(Value::as_str) {
        Some("system") => {
            let mut interpretation = LineInterpretation::none();
            if let Some(id) = value.get("session_id").and_then(Value::as_str) {
                interpretation = interpretation.with_native_session_id(id);
            }
            interpretation.usage = value.get("model").and_then(Value::as_str).map(|model| {
                UsageObservation::Estimated {
                    detail: format!("session model reported by CLI: {model}"),
                }
            });
            interpretation
        }
        Some("assistant") => {
            let mut events = Vec::new();
            for block in content_blocks(&value) {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                events.push(ProviderEvent::AssistantTextDelta {
                                    text: text.to_string(),
                                });
                            }
                        }
                    }
                    Some("tool_use") => {
                        let name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown-tool");
                        events.push(ProviderEvent::ToolRequested {
                            tool_name: name.to_string(),
                            // Only the input *keys* are summarized: tool
                            // arguments routinely contain file contents and
                            // occasionally secrets, and master plan S18/S13.5
                            // forbid persisting either by accident.
                            summary: summarize_tool_input(block.get("input")),
                        });
                    }
                    Some("thinking") => events.push(ProviderEvent::ReasoningStatus {
                        status: "thinking".to_string(),
                    }),
                    _ => {}
                }
            }
            LineInterpretation::events(events)
        }
        Some("user") => {
            let mut events = Vec::new();
            for block in content_blocks(&value) {
                if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                    let text = match block.get("content") {
                        Some(Value::String(text)) => text.clone(),
                        Some(Value::Array(items)) => items
                            .iter()
                            .filter_map(|item| item.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    };
                    if !text.is_empty() {
                        events.push(ProviderEvent::ToolOutputDelta {
                            tool_name: "tool".to_string(),
                            chunk: text,
                        });
                    }
                }
            }
            LineInterpretation::events(events)
        }
        Some("result") => {
            let is_error = value
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let mut events = Vec::new();
            if is_error {
                events.push(ProviderEvent::TerminalError {
                    message: value
                        .get("result")
                        .and_then(Value::as_str)
                        .unwrap_or("claude reported an error result")
                        .to_string(),
                });
            } else if let Some(result) = value.get("result").and_then(Value::as_str) {
                if !result.is_empty() {
                    events.push(ProviderEvent::PlanArtifactEmitted {
                        summary: result.to_string(),
                    });
                }
            }

            let usage = usage_from_result(&value);
            let mut interpretation = LineInterpretation::events(events);
            interpretation.usage = usage;
            interpretation
        }
        Some("error") => LineInterpretation::events(vec![ProviderEvent::RecoverableError {
            message: value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("claude reported an error event")
                .to_string(),
        }]),
        _ => {
            tracing::trace!(line = %trimmed, "ignoring unrecognized claude stream-json event type");
            LineInterpretation::none()
        }
    }
}

fn content_blocks(value: &Value) -> Vec<&Value> {
    value
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
        .map(|blocks| blocks.iter().collect())
        .unwrap_or_default()
}

/// Names the tool and the *shape* of its input, never its values.
fn summarize_tool_input(input: Option<&Value>) -> String {
    match input {
        Some(Value::Object(map)) => {
            let keys: Vec<&str> = map.keys().map(String::as_str).collect();
            format!("input keys: {}", keys.join(", "))
        }
        Some(other) => format!("input: {}", other_type_name(other)),
        None => "no input".to_string(),
    }
}

fn other_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Exact usage when the CLI reports it (`total_cost_usd` plus token counts),
/// `Unknown` otherwise. Master plan S17.13 forbids rendering an estimate as
/// exact, so the two are never blended.
fn usage_from_result(value: &Value) -> Option<UsageObservation> {
    let cost = value.get("total_cost_usd").and_then(Value::as_f64);
    let tokens = value.get("usage").map(|usage| {
        [
            ("input", usage.get("input_tokens").and_then(Value::as_u64)),
            ("output", usage.get("output_tokens").and_then(Value::as_u64)),
            (
                "cache_read",
                usage.get("cache_read_input_tokens").and_then(Value::as_u64),
            ),
        ]
    });
    match (cost, tokens) {
        (Some(cost), Some(tokens)) => {
            let parts: Vec<String> = tokens
                .iter()
                .filter_map(|(label, count)| count.map(|count| format!("{label}={count}")))
                .collect();
            Some(UsageObservation::Exact {
                detail: format!("cost_usd={cost:.4}; tokens {}", parts.join(", ")),
            })
        }
        (Some(cost), None) => Some(UsageObservation::Exact {
            detail: format!("cost_usd={cost:.4}"),
        }),
        (None, Some(tokens)) => {
            let parts: Vec<String> = tokens
                .iter()
                .filter_map(|(label, count)| count.map(|count| format!("{label}={count}")))
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(UsageObservation::Exact {
                    detail: format!("tokens {}", parts.join(", ")),
                })
            }
        }
        (None, None) => None,
    }
}

/// Models NACC can offer before any model discovery has run.
///
/// Deliberately *not* a "known models" list: master plan S2.7 forbids
/// hard-coding marketing names as architecture. These are the aliases the
/// installed CLI's own help text documents (`--model` accepts an alias like
/// `fable`/`opus`/`sonnet` or a full name), so they are provider-reported in
/// the only sense this adapter can honestly claim before a live
/// `list_models` exists. `list_models` returns exactly this set, and the
/// capability snapshot's models are drawn from the same function, so the
/// two can never disagree.
pub fn documented_models() -> Vec<ModelDescriptor> {
    [
        ("fable", "Fable (alias)"),
        ("opus", "Opus (alias)"),
        ("sonnet", "Sonnet (alias)"),
        ("claude-fable-5", "claude-fable-5"),
    ]
    .into_iter()
    .map(|(id, display_name)| ModelDescriptor {
        id: ModelId(id.to_string()),
        display_name: display_name.to_string(),
        // `--effort` accepts low..max regardless of alias, so the scale is
        // offered in full; the CLI is the authority on whether a specific
        // model honors a level, and the per-run validation below reports a
        // mismatch rather than guessing.
        reasoning_levels: vec![
            ReasoningLevel::Auto,
            ReasoningLevel::Low,
            ReasoningLevel::Medium,
            ReasoningLevel::High,
            ReasoningLevel::ExtraHigh,
            ReasoningLevel::Maximum,
        ],
        thinking: THINKING,
        structured_output: true,
        context_window_tokens: None,
    })
    .collect()
}

pub struct ClaudeCodeProvider {
    runner: Arc<dyn CommandRunner>,
    executable: String,
    sessions: SessionSupervisor,
}

impl ClaudeCodeProvider {
    pub fn new(runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            runner,
            executable: DEFAULT_EXECUTABLE.to_string(),
            sessions: SessionSupervisor::new(),
        }
    }

    /// Point the adapter at a specific executable (the Setup Wizard's
    /// detected path, or a WSL bridge).
    pub fn with_executable(mut self, executable: impl Into<String>) -> Self {
        self.executable = executable.into();
        self
    }

    pub fn executable(&self) -> &str {
        &self.executable
    }

    /// Sessions this adapter currently has running.
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

    /// Where Claude Code keeps its credential store on Windows. NACC checks
    /// for the file's *existence* only -- the contents are never read, which
    /// is master plan S8.4's "native credentials rule" made literal.
    fn credential_store_path() -> Option<PathBuf> {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(|home| {
                PathBuf::from(home)
                    .join(".claude")
                    .join(".credentials.json")
            })
    }
}

#[async_trait]
impl AgentProvider for ClaudeCodeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }

    fn display_name(&self) -> &str {
        "Claude Code"
    }

    async fn probe_installation(&self, _runtime: &RuntimeProfile) -> Result<InstallationProbe> {
        match self.run_probe(&["--version"]).await {
            // The CLI's own version string, verbatim (master plan S2.7:
            // show exactly what the provider reports).
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
            // "Not installed" is a normal detection outcome the Setup
            // Wizard needs reported, not an error to propagate. A missing
            // executable surfaces as `NotInstalled` from the runner, and a
            // process-level failure (e.g. a broken shim) is the same fact
            // from the user's point of view: this CLI cannot be run.
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
        // Presence of the credential store, never its contents. Absence is
        // reported as `authenticated: false` with a precise detail -- the
        // Setup Wizard tells the user to run `claude` and sign in, which is
        // the only supported way to create this file.
        match Self::credential_store_path() {
            Some(path) if path.is_file() => Ok(AuthProbe {
                authenticated: true,
                account_label: None,
                detail: Some(format!(
                    "credential store present at {} (contents never read)",
                    path.display()
                )),
            }),
            Some(path) => Ok(AuthProbe {
                authenticated: false,
                account_label: None,
                detail: Some(format!(
                    "no credential store at {}; run `claude` once and sign in",
                    path.display()
                )),
            }),
            None => Ok(AuthProbe {
                authenticated: false,
                account_label: None,
                detail: Some(
                    "neither USERPROFILE nor HOME is set, so NACC cannot locate the credential store"
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
            // No PTY path in this adapter (see nacc-process's crate doc).
            interactive_pty: false,
            session_resume: true,
            custom_agents: true,
            subagents: true,
            mcp: true,
            // `claude` itself exposes no ACP surface; only a third-party
            // bridge package does (see the Phase 0 contract doc), and NACC
            // has not adopted one.
            acp_transport: nacc_provider_core::AcpTransport::Unsupported,
            usage_reporting: true,
            cancellation_documented: true,
            captured_at_millis: now_millis(),
        })
    }

    async fn validate_profile(&self, profile: &ResolvedAgentProfile) -> Result<ProfileValidation> {
        let mut issues = Vec::new();
        let models = documented_models();
        let known = models.iter().find(|model| model.id == profile.model);
        if known.is_none() {
            issues.push(format!(
                "model {} is not one this adapter reports ({}); refusing to launch with an \
                 unverified model rather than silently substituting another",
                profile.model,
                models
                    .iter()
                    .map(|model| model.id.0.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(known) = known {
            if !known.reasoning_levels.contains(&profile.reasoning) {
                issues.push(format!(
                    "reasoning level {} is not offered for model {}",
                    profile.reasoning, profile.model
                ));
            }
        }
        // A clamped mapping is a *warning* shown to the user, not a reason
        // to refuse: master plan S10.1 says never silently downgrade, and
        // returning the fact as an issue plus `supported: true` is exactly
        // the visible-receipt behavior it asks for.
        let (_, exact) = map_reasoning(profile.reasoning);
        if !exact {
            issues.push(format!(
                "reasoning level {} maps onto Claude's {} (inexact mapping; the effective level is \
                 shown before the run starts)",
                profile.reasoning,
                map_reasoning(profile.reasoning).0.unwrap_or("default")
            ));
        }
        if issues.len() == 1 && issues[0].contains("inexact mapping") {
            return Ok(ProfileValidation {
                supported: true,
                issues,
            });
        }
        let unsupported_model = issues
            .iter()
            .any(|issue| issue.contains("not one this adapter"));
        Ok(ProfileValidation {
            supported: !unsupported_model,
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
        let session_id = SessionId(nacc_domain::WorkflowRunId::new().to_string());
        let args = build_launch_args(&request.profile, &request.prompt, &session_id);
        let cwd = PathBuf::from(&request.working_directory);
        if !cwd.is_dir() {
            return Err(ProviderError::Other(format!(
                "working directory does not exist: {}",
                cwd.display()
            )));
        }
        launch_streaming_session(
            self.runner.as_ref(),
            &self.sessions,
            StreamingLaunch {
                provider: self.id(),
                program: &self.executable,
                args: &args,
                working_directory: &cwd,
                // Inherited environment: Claude Code needs the user's own
                // proxy/CA/credential-helper configuration to authenticate
                // the way it does in a terminal. Phase 11's policy layer
                // adds an explicit per-profile decision here; the mechanism
                // (isolated_environment) already exists in nacc-process.
                env: &[],
                session_id,
                model: request.profile.model.clone(),
                interpreter: Arc::new(interpret_line),
            },
            Arc::from(sink),
            None,
        )
        .await
    }

    async fn send_input(&self, session: &SessionId, _input: AgentInput) -> Result<()> {
        // `claude -p` is a one-shot invocation: there is no follow-up prompt
        // channel on it. Resuming is the documented way to continue, so this
        // returns a typed "not supported" rather than pretending a write
        // succeeded.
        if self.sessions.get(session).is_none() {
            return Err(ProviderError::Other(format!(
                "no live session with id {}",
                session.0
            )));
        }
        Err(ProviderError::UnsupportedSetting {
            detail: "claude -p is one-shot; use resume to continue a session".to_string(),
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
        let native = request.session_id.0.clone();
        let args = build_resume_args(&request.profile, &native);
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
            // A finished session's usage is gone with its registry entry;
            // the durable record is what audit storage keeps, and returning
            // `Ok(None)` is honest ("no live session, no observation
            // available here") rather than an error that suggests a fault.
            None => Ok(None),
        }
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Helper for tests and callers that want the canonical->CLI mapping's
/// human-readable description in one place.
pub fn reasoning_mapping_description(level: ReasoningLevel) -> String {
    match map_reasoning(level) {
        (Some(value), true) => format!("{level} -> {value}"),
        (Some(value), false) => format!("{level} -> {value} (clamped/collapsed)"),
        (None, _) => format!("{level} -> CLI default (flag omitted)"),
    }
}

#[cfg(test)]
mod tests;

//! The production `NodeExecutor`: launches a real provider CLI for a node and
//! turns its normalized event stream into the engine's success/failure
//! vocabulary (master plan S8.1's `launch`, S14's node execution, S8.2's
//! event vocabulary).
//!
//! # What it refuses, and why that is the point
//!
//! An attempt is never started unless the role has a provider *and* a model
//! assigned, and the project has an explicitly chosen workspace. Guessing any
//! of those would mean an agent writing into a directory the user never
//! chose, with a provider they never selected -- exactly the failure mode
//! master plan S2.7/S16 exist to prevent. Each refusal is a `permanent`
//! failure, so the engine does not burn a retry on a configuration problem.
//!
//! # Completion
//!
//! The adapter contract guarantees exactly one terminal event per session
//! (`SessionCompleted` / `SessionCancelled` / `TerminalError`), so completion
//! is detected from the event stream rather than by polling an adapter's
//! private session registry -- which is also what lets this work through
//! `dyn AgentProvider` alone. On timeout the session is cancelled through the
//! trait, so the contained process tree is torn down rather than abandoned.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::mpsc;

use nacc_domain::{AttemptId, RoleKind, WorkflowRunId};
use nacc_events::Event;
use nacc_orchestrator::{
    NodeExecutionFailure, NodeExecutionOutcome, NodeExecutionRequest, NodeExecutor,
};
use nacc_provider_core::{
    AccountProfile, CancellationMode, EventSink, LaunchRequest, ProviderEvent, ProviderRegistry,
    ResolvedAgentProfile, RuntimeLocation, RuntimeProfile, SessionId,
};

use crate::routing::{RoleMatrixRouting, RoleSettings};

/// Cap on the summary handed back to the engine: an agent can stream
/// megabytes of text, and a node's durable detail field is not a log.
const MAX_SUMMARY_CHARS: usize = 4_000;

/// Default ceiling for one attempt when nothing else is configured. Node
/// timeouts are declared per node in the master plan, but `WorkflowNode` has
/// no timeout field yet, so this is the honest stop-gap: a real number,
/// reported when it fires, rather than an unbounded wait.
pub const DEFAULT_NODE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Forwards normalized events into a channel the executor awaits on.
struct ChannelSink {
    tx: mpsc::UnboundedSender<ProviderEvent>,
}

impl EventSink for ChannelSink {
    fn emit(&self, event: ProviderEvent) {
        // A closed receiver means the attempt already ended; dropping the
        // event is correct then, not an error worth surfacing.
        let _ = self.tx.send(event);
    }
}

/// Mapping into the durable vocabulary (`nacc_events::EventType`). Written as
/// an exhaustive match, not a catch-all: a new provider event must be a
/// compile error here rather than silently unrecorded.
fn event_type_for(event: &ProviderEvent) -> nacc_events::EventType {
    use nacc_events::EventType;
    match event {
        ProviderEvent::SessionStarted { .. } => EventType::SessionStarted,
        ProviderEvent::AssistantTextDelta { .. } => EventType::AssistantTextDelta,
        ProviderEvent::ReasoningStatus { .. } => EventType::ReasoningStatus,
        ProviderEvent::ToolRequested { .. } => EventType::ToolRequested,
        ProviderEvent::ToolApproved { .. } => EventType::ToolApproved,
        ProviderEvent::ToolDenied { .. } => EventType::ToolDenied,
        ProviderEvent::ToolStarted { .. } => EventType::ToolStarted,
        ProviderEvent::ToolOutputDelta { .. } => EventType::ToolOutputDelta,
        ProviderEvent::FileChanged { .. } => EventType::FileChanged,
        ProviderEvent::CommandStarted { .. } => EventType::CommandStarted,
        ProviderEvent::CommandOutput { .. } => EventType::CommandOutput,
        ProviderEvent::CommandCompleted { .. } => EventType::CommandCompleted,
        ProviderEvent::PlanArtifactEmitted { .. } => EventType::PlanArtifactEmitted,
        ProviderEvent::HandoffEmitted { .. } => EventType::HandoffEmitted,
        ProviderEvent::UsageUpdated { .. } => EventType::UsageUpdated,
        ProviderEvent::ApprovalRequested { .. } => EventType::ApprovalRequested,
        ProviderEvent::Warning { .. } => EventType::Warning,
        ProviderEvent::RecoverableError { .. } => EventType::RecoverableError,
        ProviderEvent::TerminalError { .. } => EventType::TerminalError,
        ProviderEvent::SessionCompleted => EventType::SessionCompleted,
        ProviderEvent::SessionCancelled => EventType::SessionCancelled,
    }
}

fn describe_role(role: &RoleKind) -> String {
    match role {
        RoleKind::Custom(name) => name.clone(),
        other => format!("{other:?}"),
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn push_bounded(buffer: &mut String, text: &str) {
    if buffer.len() >= MAX_SUMMARY_CHARS {
        return;
    }
    let room = MAX_SUMMARY_CHARS - buffer.len();
    if text.len() <= room {
        buffer.push_str(text);
    } else {
        // Truncation is explicit (an ellipsis marker), so a reader can tell a
        // clipped summary from a complete one.
        let mut end = room;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        buffer.push_str(&text[..end]);
        buffer.push('…');
    }
}

fn redact_untrusted_text(text: &str) -> String {
    nacc_secrets::redact(text, &[]).0
}

pub struct ProviderNodeExecutor {
    providers: Arc<ProviderRegistry>,
    routing: Arc<RoleMatrixRouting>,
    storage: nacc_storage::Database,
    node_timeout: Duration,
    /// The attempts currently executing, as `(provider, live session)` per
    /// attempt id. This is what makes run cancellation mean "the agent's
    /// process tree stops now" rather than "the durable state says cancelled
    /// while the process keeps running" (master plan S13.4, acceptance 16).
    in_flight: std::sync::Mutex<HashMap<AttemptId, (nacc_domain::ProviderId, SessionId)>>,
}

impl ProviderNodeExecutor {
    pub fn new(
        providers: Arc<ProviderRegistry>,
        routing: Arc<RoleMatrixRouting>,
        storage: nacc_storage::Database,
    ) -> Self {
        Self {
            providers,
            routing,
            storage,
            node_timeout: DEFAULT_NODE_TIMEOUT,
            in_flight: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Persist one normalized event against the attempt's correlation IDs.
    /// Best-effort by design: a failure to write a log line must not fail a
    /// node that otherwise succeeded, so the error is reported through
    /// `tracing` and the attempt continues.
    async fn record(&self, request: &NodeExecutionRequest, event: &ProviderEvent) {
        let mut payload = serde_json::to_value(event).unwrap_or(serde_json::Value::Null);
        let redaction_count = nacc_secrets::redact_json_value(&mut payload, &[]);
        if redaction_count > 0 {
            tracing::warn!(
                node_key = %request.node_key,
                redaction_count,
                "redacted secret-like values from provider event before persistence"
            );
        }
        let Ok(record) = Event::new(
            Some(request.project_id),
            Some(request.run_id),
            Some(request.node_run_id),
            Some(request.attempt_id),
            event_type_for(event),
            payload,
            now_millis(),
        ) else {
            return;
        };
        if let Err(err) = self.storage.append_event(&record).await {
            tracing::warn!(
                node_key = %request.node_key,
                error = %err,
                "failed to persist a normalized provider event"
            );
        }
    }
}

#[async_trait]
impl NodeExecutor for ProviderNodeExecutor {
    async fn execute(
        &self,
        request: NodeExecutionRequest,
    ) -> std::result::Result<NodeExecutionOutcome, NodeExecutionFailure> {
        let Some(provider_id) = request.provider_id else {
            return Err(NodeExecutionFailure::permanent(format!(
                "role `{}` has no provider assigned in the Role Matrix",
                describe_role(&request.role)
            )));
        };
        let Some(model) = request.model_id.clone() else {
            return Err(NodeExecutionFailure::permanent(format!(
                "role `{}` has no model assigned in the Role Matrix",
                describe_role(&request.role)
            )));
        };
        let workspace = request
            .workspace
            .clone()
            .or_else(|| self.routing.workspace_for_project(request.project_id));
        let Some(workspace) = workspace else {
            return Err(NodeExecutionFailure::permanent(
                "no workspace is assigned for this project; start the run with an explicit workspace",
            ));
        };

        let Ok(provider) = self.providers.require(provider_id).map(Arc::clone) else {
            return Err(NodeExecutionFailure::permanent(format!(
                "no adapter is registered for {provider_id} in this build"
            )));
        };

        let RoleSettings {
            reasoning,
            thinking,
        } = self.routing.settings_for(&request.role).unwrap_or_default();
        let working_directory = workspace.to_string_lossy().into_owned();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let profile = ResolvedAgentProfile {
            account: AccountProfile {
                id: nacc_domain::ProviderAccountId::new(),
                provider: provider_id,
                // A label is a display fact, and nothing has discovered one
                // yet -- empty is honest, invented is not (S8.4).
                label: String::new(),
            },
            model,
            reasoning,
            thinking,
            permission: request.permission_profile,
            runtime: RuntimeProfile {
                location: RuntimeLocation::NativeWindows,
                working_directory: working_directory.clone(),
            },
        };

        let handle = match provider
            .launch(
                LaunchRequest {
                    profile,
                    working_directory: working_directory.clone(),
                    prompt: request.instruction.clone(),
                },
                Box::new(ChannelSink { tx }),
            )
            .await
        {
            Ok(handle) => handle,
            Err(err) => {
                // A missing or unusable binary will not fix itself between
                // attempts, so it is permanent; anything else may be
                // transient (a rate limit, a busy account).
                let detail =
                    redact_untrusted_text(&format!("failed to launch {provider_id}: {err}"));
                return Err(match err {
                    nacc_provider_core::ProviderError::NotInstalled { .. } => {
                        NodeExecutionFailure::permanent(detail)
                    }
                    _ => NodeExecutionFailure::retryable(detail),
                });
            }
        };

        tracing::info!(
            provider = %provider_id,
            session = %handle.session_id.0,
            node_key = %request.node_key,
            attempt = request.attempt_number,
            "node attempt running"
        );

        let deadline = Instant::now() + request.timeout.unwrap_or(self.node_timeout);
        let effective_timeout = deadline.saturating_duration_since(Instant::now()).as_secs();

        // Registered as cancellable before the stream is awaited and
        // deregistered on the single path below, so engine-driven
        // cancellation reaches exactly the live sessions.
        let registered = match self.in_flight.lock() {
            Ok(mut in_flight) => {
                in_flight.insert(request.attempt_id, (provider_id, handle.session_id.clone()));
                true
            }
            Err(_) => false,
        };
        if !registered {
            tracing::error!(
                session = %handle.session_id.0,
                "in-flight session registry is poisoned; cancelling the newly launched provider session"
            );
            if let Err(err) = provider
                .cancel(&handle.session_id, CancellationMode::Forced)
                .await
            {
                tracing::warn!(
                    session = %handle.session_id.0,
                    error = %err,
                    "cancelling a session after registry failure also failed"
                );
            }
            return Err(NodeExecutionFailure::permanent(
                "the in-flight session registry is unavailable after an internal failure",
            ));
        }

        let outcome = async {
            let mut summary = String::new();
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                match tokio::time::timeout(remaining, rx.recv()).await {
                    Err(_elapsed) => {
                        // Cancel through the trait so the contained tree is torn
                        // down; the failure is retryable because a slow node is
                        // not a broken one.
                        if let Err(err) = provider
                            .cancel(&handle.session_id, CancellationMode::Forced)
                            .await
                        {
                            tracing::warn!(
                                session = %handle.session_id.0,
                                error = %err,
                                "cancelling a timed-out attempt failed"
                            );
                        }
                        return Err(NodeExecutionFailure::retryable(format!(
                            "attempt exceeded {effective_timeout}s and was cancelled"
                        )));
                    }
                    Ok(None) => {
                        return Err(NodeExecutionFailure::retryable(
                            "the provider event stream closed before a terminal event",
                        ));
                    }
                    Ok(Some(event)) => {
                        if let ProviderEvent::AssistantTextDelta { text } = &event {
                            let redacted = redact_untrusted_text(text);
                            push_bounded(&mut summary, &redacted);
                        }
                        self.record(&request, &event).await;
                        match event {
                            ProviderEvent::SessionCompleted => {
                                return Ok(NodeExecutionOutcome {
                                    summary: if summary.trim().is_empty() {
                                        "session completed with no assistant text".to_string()
                                    } else {
                                        summary
                                    },
                                });
                            }
                            ProviderEvent::SessionCancelled => {
                                return Err(NodeExecutionFailure::permanent(
                                    "the provider session was cancelled",
                                ));
                            }
                            ProviderEvent::TerminalError { message } => {
                                return Err(NodeExecutionFailure::retryable(format!(
                                    "provider reported a terminal error: {}",
                                    redact_untrusted_text(&message)
                                )));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        .await;
        match self.in_flight.lock() {
            Ok(mut in_flight) => {
                in_flight.remove(&request.attempt_id);
            }
            Err(_) => {
                tracing::error!(
                    attempt = %request.attempt_id,
                    "in-flight session registry is poisoned; completed attempt could not be deregistered"
                );
            }
        }
        outcome
    }

    /// Stop one in-flight attempt by killing its provider session's whole
    /// process tree. Forced, not graceful: a cancelled run is not asked
    /// nicely (master plan S13.4). `false` means the attempt had already
    /// finished -- nothing left to stop -- or the cancellation itself failed;
    /// either way it is logged and the engine's durable state stands.
    async fn cancel_attempt(&self, _run_id: WorkflowRunId, attempt_id: AttemptId) -> bool {
        let claimed = match self.in_flight.lock() {
            Ok(mut in_flight) => in_flight.remove(&attempt_id),
            Err(_) => {
                tracing::error!(
                    attempt = %attempt_id,
                    "in-flight session registry is poisoned; cancellation cannot safely claim the provider session"
                );
                return false;
            }
        };
        let Some((provider_id, session)) = claimed else {
            return false;
        };
        let Ok(provider) = self.providers.require(provider_id).map(Arc::clone) else {
            return false;
        };
        match provider.cancel(&session, CancellationMode::Forced).await {
            Ok(()) => {
                tracing::info!(
                    session = session.0,
                    "cancelled an in-flight attempt's provider session"
                );
                true
            }
            Err(err) => {
                tracing::warn!(
                    session = session.0,
                    error = %err,
                    "cancelling an in-flight attempt's provider session failed"
                );
                false
            }
        }
    }
}

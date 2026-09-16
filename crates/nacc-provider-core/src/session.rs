//! Shared session plumbing for provider adapters (master plan S8.1's
//! `launch`/`cancel`/`collect_usage` contract, S13.4's cancellation rule).
//!
//! Every adapter has to solve the same five problems around a running
//! provider CLI:
//!
//! 1. start the child, streaming its output lines as they arrive;
//! 2. emit `SessionStarted` **before** any line-derived event, no matter
//!    how fast the CLI talks;
//! 3. answer `cancel` for a session that is in flight (contained, so the
//!    whole tree dies);
//! 4. report exactly one terminal event when it ends, distinguishing
//!    cancellation from failure from success;
//! 5. remember what the provider said about usage and its own session id.
//!
//! Doing that five times would produce five subtly different answers. This
//! module does it once, and leaves adapters only what is genuinely
//! provider-specific: [`LineInterpretation`] -- a *pure* function from one
//! output line to normalized events, which is exactly the part a fixture
//! can test without any process at all.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nacc_domain::ModelId;

use crate::cli::{CommandLine, CommandLineSink, CommandRunner, CommandStream};
use crate::error::{ProviderError, Result};
use crate::events::{EventSink, ProviderEvent};
use crate::provider::{AgentSessionHandle, CancellationMode, SessionId, UsageObservation};

/// What one output line meant, as far as the adapter could tell.
#[derive(Clone, Debug, Default)]
pub struct LineInterpretation {
    /// Normalized events this line produced (often zero: most lines in a
    /// structured stream are bookkeeping the adapter deliberately ignores).
    pub events: Vec<ProviderEvent>,
    /// Usage the provider reported on this line, if any.
    pub usage: Option<UsageObservation>,
    /// The provider's own session/thread id, if this line revealed it.
    pub native_session_id: Option<String>,
}

impl LineInterpretation {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn events(events: Vec<ProviderEvent>) -> Self {
        Self {
            events,
            usage: None,
            native_session_id: None,
        }
    }

    pub fn with_native_session_id(mut self, id: impl Into<String>) -> Self {
        self.native_session_id = Some(id.into());
        self
    }

    pub fn with_usage(mut self, usage: UsageObservation) -> Self {
        self.usage = Some(usage);
        self
    }
}

/// Maps one output line to its interpretation. Pure by construction: no
/// I/O, no state -- which is why it can be tested against recorded provider
/// output directly.
pub type LineInterpreter = Arc<dyn Fn(CommandStream, &str) -> LineInterpretation + Send + Sync>;

/// How a session ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionOutcome {
    Completed { exit_code: Option<i32> },
    Failed { message: String },
    Cancelled,
}

/// The mutable, shared facts about one session: what the provider reported,
/// and how it ended.
pub struct SessionState {
    session_id: SessionId,
    usage: Mutex<Option<UsageObservation>>,
    native_session_id: Mutex<Option<String>>,
    outcome: Mutex<Option<SessionOutcome>>,
    cancelled: AtomicBool,
}

impl SessionState {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            usage: Mutex::new(None),
            native_session_id: Mutex::new(None),
            outcome: Mutex::new(None),
            cancelled: AtomicBool::new(false),
        }
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// The id the provider itself uses, once it has told us. Needed for
    /// `resume`: every documented CLI resumes by *its own* id, which is not
    /// necessarily the one NACC assigned.
    pub fn native_session_id(&self) -> Option<String> {
        self.native_session_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn set_native_session_id(&self, id: impl Into<String>) {
        *self
            .native_session_id
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(id.into());
    }

    /// Records an observation. Latest wins: a CLI that reports usage
    /// cumulatively repeats it on later lines, so the newest statement is
    /// the most complete one.
    pub fn record_usage(&self, usage: UsageObservation) {
        *self.usage.lock().unwrap_or_else(|e| e.into_inner()) = Some(usage);
    }

    pub fn usage(&self) -> Option<UsageObservation> {
        self.usage.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn outcome(&self) -> Option<SessionOutcome> {
        self.outcome
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn set_outcome(&self, outcome: SessionOutcome) {
        *self.outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// A live session: the running command plus its shared state.
pub struct SessionEntry {
    pub pid: u32,
    command: Arc<dyn crate::cli::RunningCommand>,
    state: Arc<SessionState>,
}

impl SessionEntry {
    pub fn state(&self) -> &Arc<SessionState> {
        &self.state
    }

    pub fn session_id(&self) -> &SessionId {
        self.state.session_id()
    }

    /// Stop the session's process tree (master plan S13.4). Marks the
    /// session cancelled *before* asking the process to stop, so a process
    /// that races to a non-zero exit code is still reported as cancelled
    /// rather than as a provider failure.
    pub async fn cancel(&self, mode: CancellationMode) -> Result<()> {
        self.state.cancelled.store(true, Ordering::SeqCst);
        let mode = match mode {
            CancellationMode::Graceful => nacc_process::CancelMode::Graceful,
            CancellationMode::Forced => nacc_process::CancelMode::Forced,
        };
        self.command.cancel(mode, GRACEFUL_CANCEL_WINDOW).await
    }

    /// Await the process and produce exactly one terminal event.
    pub async fn finish(&self) -> ProviderEvent {
        let outcome = match self.command.wait().await {
            Ok(output) if output.cancelled || self.state.is_cancelled() => {
                SessionOutcome::Cancelled
            }
            Ok(output) if output.succeeded() => SessionOutcome::Completed {
                exit_code: output.exit_code,
            },
            Ok(output) => SessionOutcome::Failed {
                message: format!(
                    "provider exited with code {:?}: {}",
                    output.exit_code,
                    first_non_empty(&output.stderr).unwrap_or_else(|| "(no stderr)".to_string())
                ),
            },
            Err(err) => SessionOutcome::Failed {
                message: err.to_string(),
            },
        };
        let event = match &outcome {
            SessionOutcome::Completed { .. } => ProviderEvent::SessionCompleted,
            SessionOutcome::Cancelled => ProviderEvent::SessionCancelled,
            SessionOutcome::Failed { message } => ProviderEvent::TerminalError {
                message: message.clone(),
            },
        };
        self.state.set_outcome(outcome);
        event
    }
}

/// How long a provider CLI gets to exit on its own after a graceful
/// cancellation request before its tree is terminated. Master plan S13.4
/// asks for "a policy-controlled timeout"; this is the policy's default,
/// and Phase 11's permission profiles refine it per-profile.
pub const GRACEFUL_CANCEL_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);

/// Tracks every session an adapter has launched.
///
/// `Clone` is deliberate and cheap (an `Arc` clone of the map): the task
/// that finishes a session has to remove it from the same registry the
/// adapter's `cancel`/`collect_usage` calls read, without any raw-pointer or
/// static-state shortcut.
#[derive(Clone)]
pub struct SessionSupervisor {
    sessions: Arc<Mutex<HashMap<SessionId, Arc<SessionEntry>>>>,
    /// Ids of finished sessions, oldest first, so completed runs are
    /// forgotten in a bounded way instead of growing for the app's lifetime.
    finished: Arc<Mutex<std::collections::VecDeque<SessionId>>>,
    retain_finished: usize,
}

impl Default for SessionSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionSupervisor {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            finished: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            // Enough that a workflow can finish a node, persist its usage
            // to the audit trail, and still read it back -- without keeping
            // every session of a long app run in memory forever.
            retain_finished: 32,
        }
    }

    fn insert(&self, entry: Arc<SessionEntry>) {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(entry.state.session_id().clone(), entry);
    }

    /// Record that a session ended and evict the oldest finished sessions
    /// beyond the retention limit. A finished session is *kept* rather than
    /// removed immediately, because `collect_usage` is called after a run
    /// ends (master plan S17.13) and a provider's reported cost or token
    /// count would otherwise be unreadable exactly when it is needed.
    fn mark_finished(&self, session_id: &SessionId) {
        let mut finished = self.finished.lock().unwrap_or_else(|e| e.into_inner());
        finished.push_back(session_id.clone());
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        while finished.len() > self.retain_finished {
            if let Some(oldest) = finished.pop_front() {
                sessions.remove(&oldest);
            }
        }
    }

    pub fn get(&self, session_id: &SessionId) -> Option<Arc<SessionEntry>> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(session_id)
            .cloned()
    }

    /// Cancel a session by id, with the typed "no such session" error
    /// `AgentProvider::cancel` must return for an unknown id.
    pub async fn cancel(&self, session_id: &SessionId, mode: CancellationMode) -> Result<()> {
        let entry = self.get(session_id).ok_or_else(|| {
            ProviderError::Other(format!("no live session with id {}", session_id.0))
        })?;
        // Cancelling something that already ended is not success: reporting
        // `Ok` would let a workflow believe it stopped a run that in fact
        // completed, and would make "cancel an unknown session" (which the
        // contract suite requires to be a typed error) indistinguishable
        // from cancelling a real one.
        if let Some(outcome) = entry.state().outcome() {
            return Err(ProviderError::Other(format!(
                "session {} has already finished ({outcome:?})",
                session_id.0
            )));
        }
        entry.cancel(mode).await
    }

    pub fn ids(&self) -> Vec<SessionId> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// Sessions still running (finished ones are retained but not running).
    pub fn running_count(&self) -> usize {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|entry| entry.state().outcome().is_none())
            .count()
    }

    /// Every session this supervisor still knows about, running or recently
    /// finished.
    pub fn session_count(&self) -> usize {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }
}

/// Buffers events until `SessionStarted` has been emitted, then streams
/// them. This is what makes "the first event a sink sees is
/// `SessionStarted`" true even when a CLI's first stdout line arrives
/// before `spawn()` has even returned -- which is exactly what happens with
/// a fast, fixture-backed, or cached process.
struct StartedSink {
    sink: Arc<dyn EventSink>,
    interpreter: LineInterpreter,
    state: Arc<SessionState>,
    started: AtomicBool,
    buffer: Mutex<Vec<ProviderEvent>>,
}

impl StartedSink {
    fn emit(&self, event: ProviderEvent) {
        if self.started.load(Ordering::SeqCst) {
            self.sink.emit(event);
        } else {
            self.buffer
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(event);
        }
    }

    fn apply(&self, interpretation: LineInterpretation) {
        if let Some(native) = interpretation.native_session_id {
            self.state.set_native_session_id(native);
        }
        if let Some(usage) = interpretation.usage {
            self.state.record_usage(usage);
        }
        for event in interpretation.events {
            self.emit(event);
        }
    }

    /// Emit `SessionStarted`, then everything buffered so far. Called with
    /// the id the provider announced if it already has, and NACC's own id
    /// otherwise.
    fn mark_started(&self, session_id: &SessionId, model: &ModelId) {
        let provider_session_id = self
            .state
            .native_session_id()
            .unwrap_or_else(|| session_id.0.clone());
        self.started.store(true, Ordering::SeqCst);
        self.sink.emit(ProviderEvent::SessionStarted {
            provider_session_id,
            model: model.clone(),
        });
        let buffered = {
            let mut buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *buffer)
        };
        for event in buffered {
            self.sink.emit(event);
        }
    }
}

impl CommandLineSink for StartedSink {
    fn line(&self, line: CommandLine) {
        let interpretation = (self.interpreter)(line.stream, &line.text);
        self.apply(interpretation);
    }
}

/// What a caller needs to launch a streaming session: everything the
/// adapter resolved, plus the pure line interpreter.
pub struct StreamingLaunch<'a> {
    pub provider: nacc_domain::ProviderId,
    pub program: &'a str,
    pub args: &'a [String],
    pub working_directory: &'a Path,
    pub env: &'a [(String, String)],
    pub session_id: SessionId,
    pub model: ModelId,
    pub interpreter: LineInterpreter,
}

/// Spawn a provider CLI, stream its normalized events, and return the
/// session handle immediately (the run continues in the background).
///
/// `on_finish` is invoked once, after the terminal event is emitted; the
/// orchestrator uses it to advance a node in the durable state machine
/// (master plan S14) without the adapter knowing anything about workflows.
pub async fn launch_streaming_session(
    runner: &dyn CommandRunner,
    registry: &SessionSupervisor,
    launch: StreamingLaunch<'_>,
    sink: Arc<dyn EventSink>,
    on_finish: Option<Arc<dyn Fn(ProviderEvent) + Send + Sync>>,
) -> Result<AgentSessionHandle> {
    let model = launch.model.clone();
    let session_id = launch.session_id.clone();
    let state = Arc::new(SessionState::new(session_id.clone()));
    let starting_sink = Arc::new(StartedSink {
        sink: Arc::clone(&sink),
        interpreter: launch.interpreter,
        state: Arc::clone(&state),
        started: AtomicBool::new(false),
        buffer: Mutex::new(Vec::new()),
    });

    let command = runner
        .spawn(
            launch.program,
            launch.args,
            launch.working_directory,
            launch.env,
            Arc::clone(&starting_sink) as Arc<dyn CommandLineSink>,
        )
        .await?;

    // Now that the process exists, "started" is a fact rather than a hope.
    starting_sink.mark_started(&session_id, &model);

    let entry = Arc::new(SessionEntry {
        pid: command.pid(),
        command,
        state,
    });
    registry.insert(Arc::clone(&entry));
    let registry_for_task = registry.clone();

    tokio::spawn(async move {
        let terminal = entry.finish().await;
        tracing::debug!(
            session_id = %entry.session_id().0,
            outcome = ?entry.state().outcome(),
            "provider session finished"
        );
        registry_for_task.mark_finished(entry.session_id());
        sink.emit(terminal.clone());
        if let Some(on_finish) = on_finish {
            on_finish(terminal);
        }
    });

    Ok(AgentSessionHandle {
        session_id,
        provider: launch.provider,
    })
}

fn first_non_empty(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

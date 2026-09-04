//! Policy-bound process/PTY supervision for native coding-agent sessions.
//!
//! This is Phase 3's execution primitive, not a webview command surface.
//! Higher layers must resolve provider/runtime/policy decisions before they
//! construct a [`ProcessSpec`]; this crate deliberately exposes no Tauri IPC
//! and never accepts a shell command string to interpolate. Programs and
//! arguments remain separate values, the child environment is cleared and
//! rebuilt from an explicit allowlist, and terminal output is carried as raw
//! bytes so partial UTF-8/ANSI sequences cannot be corrupted.
//!
//! On Windows every spawned child is assigned to a Job Object configured with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Explicit cancellation uses
//! `TerminateJobObject`, while closing the last job handle remains a crash-safe
//! backstop. PTY creation + child spawn are serialized because the audited
//! AgentPanel foundation demonstrated a real ConPTY concurrent-spawn stall.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

const DEFAULT_EVENT_CAPACITY: usize = 64;
const READ_BUFFER_BYTES: usize = 8 * 1024;
const BACKPRESSURE_POLL: Duration = Duration::from_millis(2);

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("invalid process specification: {0}")]
    InvalidSpec(String),
    #[error("PTY operation failed: {0}")]
    Pty(String),
    #[error("process supervisor state lock was poisoned: {0}")]
    StatePoisoned(&'static str),
    #[error("process session {0} does not exist")]
    SessionNotFound(ProcessSessionId),
    #[error("spawned PTY child did not expose a process id/handle")]
    MissingProcessIdentity,
    #[error("I/O operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Windows Job Object operation failed: {0}")]
    Job(String),
}

pub type Result<T> = std::result::Result<T, ProcessError>;

/// Opaque identifier for one live process/PTY session.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProcessSessionId(u64);

impl ProcessSessionId {
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ProcessSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Fully resolved launch request supplied by a trusted provider/runtime layer.
///
/// `program` is an executable, never an interpolated shell command. `args` are
/// passed separately. `environment` is the complete child environment: the
/// inherited process environment is intentionally cleared before these values
/// are applied, so callers must opt in to PATH, HOME/USERPROFILE, provider
/// config locations, and any other variable their provider requires.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    pub rows: u16,
    pub cols: u16,
}

impl ProcessSpec {
    pub fn validate(&self) -> Result<()> {
        if self.program.trim().is_empty() {
            return Err(ProcessError::InvalidSpec(
                "program must not be empty".to_string(),
            ));
        }
        if self.program.contains('\0') || self.args.iter().any(|arg| arg.contains('\0')) {
            return Err(ProcessError::InvalidSpec(
                "program and arguments must not contain NUL bytes".to_string(),
            ));
        }
        if self.rows == 0 || self.cols == 0 {
            return Err(ProcessError::InvalidSpec(
                "PTY rows and columns must both be greater than zero".to_string(),
            ));
        }
        if let Some(cwd) = &self.cwd {
            if !cwd.is_dir() {
                return Err(ProcessError::InvalidSpec(format!(
                    "working directory does not exist or is not a directory: {}",
                    cwd.display()
                )));
            }
        }
        for (name, value) in &self.environment {
            if name.trim().is_empty() || name.contains('=') || name.contains('\0') {
                return Err(ProcessError::InvalidSpec(format!(
                    "invalid environment variable name: {name:?}"
                )));
            }
            if value.contains('\0') {
                return Err(ProcessError::InvalidSpec(format!(
                    "environment variable {name:?} contains a NUL byte"
                )));
            }
        }
        Ok(())
    }
}

/// Normalized low-level events from one supervised process.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessEvent {
    Output { bytes: Vec<u8> },
    Exited { exit_code: Option<u32> },
    Cancelled,
    ReaderFailed { message: String },
}

pub struct SpawnedProcess {
    pub id: ProcessSessionId,
    pub events: mpsc::Receiver<ProcessEvent>,
}

struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    job: Option<platform::ProcessJob>,
    events: mpsc::Sender<ProcessEvent>,
    cancelled: Arc<AtomicBool>,
}

impl PtySession {
    fn terminate_tree(&mut self) -> Result<()> {
        self.cancelled.store(true, Ordering::Release);

        let job_result = if let Some(job) = self.job.as_ref() {
            job.terminate()
        } else {
            Ok(())
        };

        // The Job Object is authoritative on Windows. Child::kill is retained
        // as a direct-child fallback on non-Windows and as harmless belt-and-
        // braces cleanup if a process exited during cancellation.
        let child_kill_result = self.child.kill();
        let _ = self.child.wait();
        self.job.take(); // kill-on-close remains the final Windows backstop.

        job_result?;
        if cfg!(not(windows)) {
            child_kill_result?;
        }
        Ok(())
    }
}

/// Owns all live PTY sessions for one application process.
///
/// No Tauri command is defined here. The future Tauri composition root must
/// expose only policy-checked commands which call into this supervisor.
pub struct ProcessSupervisor {
    // Audited AgentPanel mitigation: ConPTY can stall an output pipe when two
    // openpty+spawn sequences happen concurrently.
    spawn_lock: Mutex<()>,
    sessions: Arc<Mutex<HashMap<ProcessSessionId, PtySession>>>,
    next_id: AtomicU64,
    event_capacity: usize,
}

impl Default for ProcessSupervisor {
    fn default() -> Self {
        Self::new(DEFAULT_EVENT_CAPACITY)
    }
}

impl ProcessSupervisor {
    pub fn new(event_capacity: usize) -> Self {
        Self {
            spawn_lock: Mutex::new(()),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(0),
            event_capacity: event_capacity.max(1),
        }
    }

    pub fn spawn(&self, spec: ProcessSpec) -> Result<SpawnedProcess> {
        spec.validate()?;
        let _spawn_guard = self.lock_spawn()?;

        let job = platform::ProcessJob::new()?;
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: spec.rows,
                cols: spec.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| ProcessError::Pty(format!("openpty: {e}")))?;

        let mut command = CommandBuilder::new(&spec.program);
        command.env_clear();
        for arg in &spec.args {
            command.arg(arg);
        }
        if let Some(cwd) = &spec.cwd {
            command.cwd(cwd);
        }
        for (name, value) in &spec.environment {
            command.env(name, value);
        }

        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| ProcessError::Pty(format!("spawn child: {e}")))?;

        if child.process_id().is_none() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProcessError::MissingProcessIdentity);
        }
        if let Err(error) = job.assign_child(child.as_ref()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }

        // The slave must be closed in the parent so EOF can propagate when the
        // child exits; portable-pty's own examples call this out explicitly.
        drop(pair.slave);

        let reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(error) => {
                let _ = job.terminate();
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProcessError::Pty(format!("clone PTY reader: {error}")));
            }
        };
        let writer = match pair.master.take_writer() {
            Ok(writer) => writer,
            Err(error) => {
                let _ = job.terminate();
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProcessError::Pty(format!("take PTY writer: {error}")));
            }
        };

        let id = ProcessSessionId(self.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let (events, receiver) = mpsc::channel(self.event_capacity);
        let cancelled = Arc::new(AtomicBool::new(false));

        // Insert before the reader starts: a very short-lived command may hit
        // EOF immediately and must still be able to claim its session for wait.
        self.lock_sessions()?.insert(
            id,
            PtySession {
                master: pair.master,
                writer,
                child,
                job: Some(job),
                events: events.clone(),
                cancelled: Arc::clone(&cancelled),
            },
        );

        let sessions = Arc::clone(&self.sessions);
        thread::Builder::new()
            .name(format!("nacc-pty-reader-{}", id.as_u64()))
            .spawn(move || reader_loop(id, reader, events, cancelled, sessions))
            .map_err(|error| {
                // If even starting the drain thread fails, remove and terminate
                // the child immediately; leaving a PTY without a reader can
                // deadlock a verbose coding agent on a full output buffer.
                if let Ok(mut sessions) = self.sessions.lock() {
                    if let Some(mut session) = sessions.remove(&id) {
                        let _ = session.terminate_tree();
                    }
                }
                ProcessError::Io(error)
            })?;

        Ok(SpawnedProcess {
            id,
            events: receiver,
        })
    }

    pub fn write(&self, id: ProcessSessionId, bytes: &[u8]) -> Result<()> {
        let mut sessions = self.lock_sessions()?;
        let session = sessions
            .get_mut(&id)
            .ok_or(ProcessError::SessionNotFound(id))?;
        session.writer.write_all(bytes)?;
        session.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, id: ProcessSessionId, rows: u16, cols: u16) -> Result<()> {
        if rows == 0 || cols == 0 {
            return Err(ProcessError::InvalidSpec(
                "PTY rows and columns must both be greater than zero".to_string(),
            ));
        }
        let sessions = self.lock_sessions()?;
        let session = sessions
            .get(&id)
            .ok_or(ProcessError::SessionNotFound(id))?;
        session
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| ProcessError::Pty(format!("resize PTY: {e}")))
    }

    pub fn cancel(&self, id: ProcessSessionId) -> Result<()> {
        let mut session = self
            .lock_sessions()?
            .remove(&id)
            .ok_or(ProcessError::SessionNotFound(id))?;
        session.cancelled.store(true, Ordering::Release);
        let _ = session.events.try_send(ProcessEvent::Cancelled);
        session.terminate_tree()
    }

    pub fn active_count(&self) -> Result<usize> {
        Ok(self.lock_sessions()?.len())
    }

    pub fn is_active(&self, id: ProcessSessionId) -> Result<bool> {
        Ok(self.lock_sessions()?.contains_key(&id))
    }

    /// Cancel every live session. All sessions are removed from the registry
    /// even if one cancellation reports an error, so a single bad child cannot
    /// prevent later Job handles from being closed.
    pub fn shutdown_all(&self) -> Result<usize> {
        let sessions: Vec<_> = self.lock_sessions()?.drain().collect();
        let count = sessions.len();
        let mut first_error = None;

        for (_id, mut session) in sessions {
            session.cancelled.store(true, Ordering::Release);
            let _ = session.events.try_send(ProcessEvent::Cancelled);
            if let Err(error) = session.terminate_tree() {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }

        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(count)
        }
    }

    fn lock_spawn(&self) -> Result<MutexGuard<'_, ()>> {
        self.spawn_lock
            .lock()
            .map_err(|_| ProcessError::StatePoisoned("spawn_lock"))
    }

    fn lock_sessions(&self) -> Result<MutexGuard<'_, HashMap<ProcessSessionId, PtySession>>> {
        self.sessions
            .lock()
            .map_err(|_| ProcessError::StatePoisoned("sessions"))
    }
}

impl Drop for ProcessSupervisor {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            for (_id, mut session) in sessions.drain() {
                session.cancelled.store(true, Ordering::Release);
                let _ = session.terminate_tree();
            }
        }
    }
}

fn reader_loop(
    id: ProcessSessionId,
    mut reader: Box<dyn Read + Send>,
    events: mpsc::Sender<ProcessEvent>,
    cancelled: Arc<AtomicBool>,
    sessions: Arc<Mutex<HashMap<ProcessSessionId, PtySession>>>,
) {
    let mut buffer = [0_u8; READ_BUFFER_BYTES];
    let mut sink_open = true;

    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if sink_open {
                    let sent = send_with_backpressure(
                        &events,
                        ProcessEvent::Output {
                            bytes: buffer[..read].to_vec(),
                        },
                        &cancelled,
                    );
                    if !sent {
                        sink_open = false;
                        if cancelled.load(Ordering::Acquire) {
                            break;
                        }
                    }
                }
                // If the receiver disappeared, continue draining the PTY until
                // EOF. Stopping reads solely because the UI detached can fill
                // the PTY buffer and deadlock the child.
            }
            Err(error) => {
                if sink_open {
                    let _ = events.try_send(ProcessEvent::ReaderFailed {
                        message: error.to_string(),
                    });
                }
                let removed = sessions
                    .lock()
                    .ok()
                    .and_then(|mut sessions| sessions.remove(&id));
                if let Some(mut session) = removed {
                    if let Err(terminate_error) = session.terminate_tree() {
                        tracing::warn!(
                            session_id = id.as_u64(),
                            error = %terminate_error,
                            "failed to terminate PTY job after reader failure"
                        );
                    }
                }
                return;
            }
        }
    }

    // Natural EOF owns the wait/reap path. Intentional cancellation removes
    // the session before killing it, so `remove` returns None and no duplicate
    // Exited event is emitted after Cancelled.
    let removed = sessions
        .lock()
        .ok()
        .and_then(|mut sessions| sessions.remove(&id));
    if let Some(mut session) = removed {
        let exit_code = session.child.wait().ok().map(|status| status.exit_code());
        if sink_open {
            let _ = send_with_backpressure(
                &events,
                ProcessEvent::Exited { exit_code },
                &cancelled,
            );
        }
    }
}

fn send_with_backpressure(
    sender: &mpsc::Sender<ProcessEvent>,
    mut event: ProcessEvent,
    cancelled: &AtomicBool,
) -> bool {
    loop {
        if cancelled.load(Ordering::Acquire) {
            return false;
        }
        match sender.try_send(event) {
            Ok(()) => return true,
            Err(mpsc::error::TrySendError::Full(returned)) => {
                event = returned;
                thread::sleep(BACKPRESSURE_POLL);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
        }
    }
}

#[cfg(windows)]
mod platform {
    use portable_pty::Child;
    use win32job::{ExtendedLimitInfo, Job};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::TerminateJobObject;

    use super::{ProcessError, Result};

    pub struct ProcessJob(Job);

    impl ProcessJob {
        pub fn new() -> Result<Self> {
            let mut limits = ExtendedLimitInfo::new();
            limits.limit_kill_on_job_close();
            Job::create_with_limit_info(&limits)
                .map(Self)
                .map_err(|e| ProcessError::Job(format!("create/configure job: {e}")))
        }

        pub fn assign_child(&self, child: &(dyn Child + Send + Sync)) -> Result<()> {
            let handle = child
                .as_raw_handle()
                .ok_or(ProcessError::MissingProcessIdentity)?;
            self.0
                .assign_process(handle as isize)
                .map_err(|e| ProcessError::Job(format!("assign process to job: {e}")))
        }

        pub fn terminate(&self) -> Result<()> {
            unsafe { TerminateJobObject(HANDLE(self.0.handle() as _), 1) }
                .map_err(|e| ProcessError::Job(format!("terminate job: {e}")))
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use portable_pty::Child;

    use super::Result;

    pub struct ProcessJob;

    impl ProcessJob {
        pub fn new() -> Result<Self> {
            Ok(Self)
        }

        pub fn assign_child(&self, _child: &(dyn Child + Send + Sync)) -> Result<()> {
            Ok(())
        }

        pub fn terminate(&self) -> Result<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_spec(program: &str) -> ProcessSpec {
        ProcessSpec {
            program: program.to_string(),
            args: Vec::new(),
            cwd: None,
            environment: BTreeMap::new(),
            rows: 24,
            cols: 80,
        }
    }

    #[test]
    fn rejects_empty_program_and_zero_dimensions() {
        let mut spec = valid_spec(" ");
        assert!(matches!(spec.validate(), Err(ProcessError::InvalidSpec(_))));
        spec.program = "tool".to_string();
        spec.rows = 0;
        assert!(matches!(spec.validate(), Err(ProcessError::InvalidSpec(_))));
    }

    #[test]
    fn rejects_unsafe_environment_names() {
        let mut spec = valid_spec("tool");
        spec.environment.insert("BAD=NAME".to_string(), "x".to_string());
        assert!(matches!(spec.validate(), Err(ProcessError::InvalidSpec(_))));
    }

    #[test]
    fn event_capacity_is_never_zero() {
        let supervisor = ProcessSupervisor::new(0);
        assert_eq!(supervisor.event_capacity, 1);
    }

    #[cfg(windows)]
    fn comspec() -> String {
        std::env::var("COMSPEC").unwrap_or_else(|_| "C:\\Windows\\System32\\cmd.exe".to_string())
    }

    #[cfg(windows)]
    #[test]
    fn windows_spawn_streams_raw_output_and_reports_exit() {
        let supervisor = ProcessSupervisor::default();
        let mut spec = valid_spec(&comspec());
        spec.args = vec![
            "/D".to_string(),
            "/Q".to_string(),
            "/C".to_string(),
            "echo nacc-process-ok".to_string(),
        ];
        let mut spawned = supervisor.spawn(spec).expect("spawn cmd.exe in ConPTY");

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut output = Vec::new();
        let mut exit_code = None;
        while std::time::Instant::now() < deadline {
            match spawned.events.try_recv() {
                Ok(ProcessEvent::Output { bytes }) => output.extend(bytes),
                Ok(ProcessEvent::Exited { exit_code: code }) => {
                    exit_code = code;
                    break;
                }
                Ok(ProcessEvent::ReaderFailed { message }) => {
                    panic!("reader failed: {message}")
                }
                Ok(ProcessEvent::Cancelled) => panic!("process was unexpectedly cancelled"),
                Err(mpsc::error::TryRecvError::Empty) => thread::sleep(Duration::from_millis(10)),
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        assert_eq!(exit_code, Some(0));
        assert!(
            output.windows(b"nacc-process-ok".len()).any(|w| w == b"nacc-process-ok"),
            "terminal output did not contain marker: {:?}",
            String::from_utf8_lossy(&output)
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_cancel_removes_live_session_and_terminates_job() {
        let supervisor = ProcessSupervisor::default();
        let mut spec = valid_spec(&comspec());
        spec.args = vec![
            "/D".to_string(),
            "/Q".to_string(),
            "/C".to_string(),
            "ping -t 127.0.0.1 > nul".to_string(),
        ];
        let spawned = supervisor.spawn(spec).expect("spawn long-running cmd.exe");
        assert!(supervisor.is_active(spawned.id).expect("query active state"));

        supervisor.cancel(spawned.id).expect("cancel job");
        assert!(!supervisor.is_active(spawned.id).expect("query active state"));
    }
}

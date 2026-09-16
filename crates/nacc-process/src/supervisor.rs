//! Process supervision: spawn a provider CLI under a Windows Job Object,
//! stream its output line by line, and cancel the whole tree on demand
//! (master plan S10, S13.4, S16).
//!
//! Design decisions worth stating plainly:
//!
//! - **No window, ever** (`CREATE_NO_WINDOW`). A provider CLI spawned from
//!   a GUI must not flash a console window, and master plan S13.1 requires
//!   the webview never to gain a general shell capability -- so the only
//!   process creation that exists anywhere in NACC is this crate, driven
//!   from Rust, with an argument array rather than a shell string
//!   (S13.3).
//! - **Dropping a [`SupervisedProcess`] kills its tree.** The job object is
//!   configured with `KILL_ON_JOB_CLOSE`; since the app holds the last
//!   handle, a NACC crash or an accidental dropped handle terminates the
//!   provider tree at the OS level. Losing a handle must never orphan a
//!   process that keeps writing to a worktree.
//! - **Graceful cancellation closes stdin.** With no console attached
//!   (deliberately) there is no process group to send `CTRL_BREAK` to, so
//!   the available graceful signal for a piped CLI is end-of-input on
//!   stdin; if the process has not exited within the caller's grace
//!   period, the job is terminated. This is exactly master plan S13.4's
//!   two-step policy (graceful request, then bounded wait, then forced
//!   termination), made concrete instead of aspirational.
//! - **Every output line is delivered before `wait()` returns**, with one
//!   documented exception: a grandchild process that inherited the stdout
//!   pipe can hold it open after the direct child exits. Rather than hang
//!   forever on that, the reader tasks are given a bounded linger time and
//!   the situation is logged.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;

use crate::containment::{self, JobObject};

/// `CREATE_NO_WINDOW` -- the child gets no console at all, so nothing can
/// flash on screen from a GUI-launched run.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How long `wait()` will keep waiting for the output readers after the
/// direct child has exited. Only matters when a grandchild inherited the
/// stdout/stderr pipe and is still holding it open (see the module doc);
/// long enough for a normal CLI's final output to flush, short enough that
/// a stuck pipe cannot stall a workflow node forever.
const OUTPUT_LINGER: Duration = Duration::from_secs(2);

/// Which stream a line came from.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStream {
    Stdout,
    Stderr,
}

/// One line of process output, already split on line boundaries. A
/// provider adapter maps these onto `nacc_provider_core::ProviderEvent`s;
/// the line framing itself is deliberately provider-agnostic.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct ProcessLine {
    pub stream: ProcessStream,
    pub text: String,
}

/// Receives every output line a supervised process produces. A trait (not
/// a channel type) so provider adapters and the orchestrator can each plug
/// in their own consumer without this crate depending on either.
pub trait LineSink: Send + Sync {
    fn line(&self, line: ProcessLine);
}

/// Drops every line. Useful where only the exit status matters.
pub struct DiscardLines;

impl LineSink for DiscardLines {
    fn line(&self, _line: ProcessLine) {}
}

/// Logs every line at TRACE level. Master plan S8.2 permits *retaining*
/// raw provider streams in diagnostics as long as they are redacted; the
/// redaction half is Phase 11's `nacc-secrets` work, so this sink's own
/// doc comment says plainly that it must not be pointed at a run whose
/// output may contain a credential until that layer exists. It is here, and
/// used by tests, so the retention path is exercised rather than theoretical.
pub struct TracingLineSink;

impl LineSink for TracingLineSink {
    fn line(&self, line: ProcessLine) {
        match line.stream {
            ProcessStream::Stdout => tracing::trace!(line = %line.text, "process stdout"),
            ProcessStream::Stderr => tracing::trace!(line = %line.text, "process stderr"),
        }
    }
}

/// How a running process should be stopped (master plan S13.4).
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CancelMode {
    /// Close stdin and give the process `grace` to exit on its own.
    Graceful,
    /// Terminate the whole contained tree immediately.
    Forced,
}

/// Exactly what to spawn. A struct rather than a pile of arguments because
/// every field is audited (master plan S22 records the executable and the
/// redacted argument list), so the caller should build it deliberately.
#[derive(Clone, Debug)]
pub struct ProcessSpec {
    /// Executable path or name. Never a shell string: `args` is passed as
    /// an argument array (S13.3), so nothing here is subject to shell
    /// metacharacter interpretation.
    pub program: String,
    pub args: Vec<String>,
    pub working_directory: PathBuf,
    /// Extra environment variables layered on top of the inherited
    /// environment (e.g. `WSL_UTF8=1` for `wsl.exe`, or a provider's
    /// documented project-root override).
    pub env: Vec<(String, String)>,
    /// Human-readable label for logs and audit records.
    pub label: String,
}

impl ProcessSpec {
    pub fn new(program: impl Into<String>, working_directory: impl Into<PathBuf>) -> Self {
        let program = program.into();
        Self {
            label: program.clone(),
            program,
            args: Vec::new(),
            working_directory: working_directory.into(),
            env: Vec::new(),
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
}

/// Everything a short-lived command produced. Returned by
/// [`ProcessSupervisor::capture`] for the many uses where a caller wants
/// the whole output at once rather than a line stream (a `--version`
/// probe, a JSON-mode run whose result is parsed at the end).
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct CapturedOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit: ProcessExit,
}

impl CapturedOutput {
    pub fn succeeded(&self) -> bool {
        self.exit.succeeded()
    }
}

/// How a supervised process ended.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct ProcessExit {
    /// The OS exit code, or `None` when the process was killed by a signal
    /// (on Windows: terminated without a code, e.g. by the OS).
    pub exit_code: Option<i32>,
    /// Whether NACC itself asked for this process to stop. A cancelled run
    /// must never be reported as a plain failure (master plan S14.5's
    /// repair policy distinguishes the two).
    pub cancelled: bool,
}

impl ProcessExit {
    pub fn succeeded(&self) -> bool {
        !self.cancelled && self.exit_code == Some(0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("failed to spawn `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("the spawned process reported no process id")]
    NoProcessId,
    #[error("process containment failed: {0}")]
    Containment(#[from] crate::containment::ContainmentError),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ProcessError>;

/// Spawns and contains supervised processes. Stateless today (each spawn
/// owns its own job object), which is why it is a unit struct rather than a
/// singleton -- the orchestrator holds one and adds whatever bookkeeping it
/// needs around it.
#[derive(Default)]
pub struct ProcessSupervisor;

impl ProcessSupervisor {
    pub fn new() -> Self {
        Self
    }

    /// Spawn `spec` inside a fresh Job Object and start streaming its
    /// output to `sink`.
    pub async fn spawn(
        &self,
        spec: ProcessSpec,
        sink: Arc<dyn LineSink>,
    ) -> Result<SupervisedProcess> {
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .current_dir(&spec.working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &spec.env {
            command.env(key, value);
        }
        #[cfg(windows)]
        {
            // `tokio::process::Command` exposes `creation_flags` natively on
            // Windows (no `std::os::windows::process::CommandExt` needed).
            command.creation_flags(CREATE_NO_WINDOW);
        }

        tracing::debug!(
            label = %spec.label,
            program = %spec.program,
            arg_count = spec.args.len(),
            working_directory = %spec.working_directory.display(),
            "spawning contained process"
        );

        let mut child = command.spawn().map_err(|source| ProcessError::Spawn {
            program: spec.program.clone(),
            source,
        })?;
        let pid = child.id().ok_or(ProcessError::NoProcessId)?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // Contain FIRST, stream second. If containment cannot be
        // established, kill the child rather than run a provider tree
        // outside a job: master plan S13.4 makes containment a requirement,
        // not best-effort, and a provider CLI that spawns helpers without
        // it is exactly the orphan case S16 forbids.
        let job = match self.contain(&mut child, pid) {
            Ok(job) => job,
            Err(err) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(err);
            }
        };

        let mut readers = Vec::new();
        if let Some(stdout) = stdout {
            readers.push(spawn_reader(stdout, ProcessStream::Stdout, Arc::clone(&sink)));
        }
        if let Some(stderr) = stderr {
            readers.push(spawn_reader(stderr, ProcessStream::Stderr, Arc::clone(&sink)));
        }

        // The channel value is "the exit code once known": `None` means
        // still running. It is explicitly typed because a bare
        // `watch::channel(None)` infers `Option<Option<i32>>`, which then
        // does not match the receiver's declared type.
        let (exit_tx, exit_rx) = watch::channel::<Option<i32>>(None);
        let watcher = tokio::spawn(async move {
            let status = child.wait().await;
            let code = status.ok().and_then(|s| s.code());
            tracing::debug!(pid, exit_code = ?code, "contained process exited");
            // A dropped receiver is normal (nobody waited); not an error.
            let _ = exit_tx.send(code);
        });

        Ok(SupervisedProcess {
            pid,
            label: spec.label,
            job,
            stdin: Arc::new(Mutex::new(stdin)),
            exit_rx,
            readers: Mutex::new(readers),
            watcher: Mutex::new(Some(watcher)),
            cancelled: AtomicBool::new(false),
        })
    }

    /// Run `spec` to completion, returning all of its output. Lines are
    /// re-joined with `\n`: a trailing newline (or its absence) is not
    /// preserved, which is deliberate -- callers parse structured output
    /// (JSON Lines, a version string) where trailing-whitespace fidelity is
    /// meaningless, and preserving byte-exact framing would require a
    /// second, raw path this crate does not need yet.
    pub async fn capture(&self, spec: ProcessSpec) -> Result<CapturedOutput> {
        let sink = Arc::new(BufferSink::default());
        let process = self.spawn(spec, sink.clone()).await?;
        let exit = process.wait().await?;
        let (stdout, stderr) = sink.take_joined();
        Ok(CapturedOutput {
            stdout,
            stderr,
            exit,
        })
    }

    /// Create and attach a Job Object for an already-spawned child. On
    /// non-Windows targets (unsupported in this build, see
    /// `containment`'s module doc) this degrades to `None` with a warning,
    /// so the caller knows containment is weaker rather than believing it
    /// happened.
    fn contain(&self, child: &mut Child, pid: u32) -> Result<Option<Arc<JobObject>>> {
        match JobObject::create() {
            Ok(job) => {
                #[cfg(windows)]
                {
                    let handle = child.raw_handle().ok_or(ProcessError::NoProcessId)?;
                    job.assign(pid, handle as usize)?;
                }
                #[cfg(not(windows))]
                {
                    let _ = child;
                }
                Ok(Some(Arc::new(job)))
            }
            Err(containment::ContainmentError::UnsupportedPlatform) => {
                tracing::warn!(
                    pid,
                    "process containment unavailable on this platform; only the direct child can be terminated"
                );
                Ok(None)
            }
            Err(err) => Err(ProcessError::Containment(err)),
        }
    }
}

fn spawn_reader<R>(
    reader: R,
    stream: ProcessStream,
    sink: Arc<dyn LineSink>,
) -> JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(text)) => sink.line(ProcessLine { stream, text }),
                Ok(None) => break,
                Err(err) => {
                    // Every provider CLI NACC adapts emits UTF-8; a decoding
                    // error means a partial/invalid line, so drop the rest
                    // of that stream loudly instead of fabricating a line
                    // that would flow into the normalized event vocabulary
                    // as if the provider had said it.
                    tracing::warn!(?stream, error = %err, "process output stream ended on a decode error; remaining bytes discarded");
                    break;
                }
            }
        }
    })
}

/// A live, contained process. Hold this for the run's duration: dropping it
/// terminates the tree (see the module doc).
pub struct SupervisedProcess {
    pid: u32,
    label: String,
    job: Option<Arc<JobObject>>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    exit_rx: watch::Receiver<Option<i32>>,
    readers: Mutex<Vec<JoinHandle<()>>>,
    watcher: Mutex<Option<JoinHandle<()>>>,
    cancelled: AtomicBool,
}

impl SupervisedProcess {
    /// The OS process id of the direct child (not the job object).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Whether NACC has asked this process to stop.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Whether this process is contained in a real Job Object (i.e. whether
    /// cancellation reaches descendants).
    pub fn is_contained(&self) -> bool {
        self.job.is_some()
    }

    /// Write to the child's stdin (e.g. a follow-up prompt). Fails if the
    /// process has already closed its input end.
    pub async fn write_stdin(&self, data: &[u8]) -> Result<()> {
        let mut guard = self.stdin.lock().await;
        let stdin = guard
            .as_mut()
            .ok_or_else(|| ProcessError::Other("process stdin is already closed".into()))?;
        stdin.write_all(data).await.map_err(|e| {
            ProcessError::Other(format!("failed to write to process stdin: {e}"))
        })?;
        stdin
            .flush()
            .await
            .map_err(|e| ProcessError::Other(format!("failed to flush process stdin: {e}")))
    }

    /// Close the child's stdin. This is both the graceful-cancel signal and
    /// the normal way to end a provider's interactive-but-piped session.
    pub async fn close_stdin(&self) -> Result<()> {
        let mut guard = self.stdin.lock().await;
        if let Some(mut stdin) = guard.take() {
            let _ = stdin.shutdown().await;
        }
        Ok(())
    }

    /// Stop the process (master plan S13.4). `Graceful` closes stdin and
    /// waits up to `grace`; `Forced` terminates the tree immediately. Both
    /// are idempotent and safe to call concurrently with [`Self::wait`].
    pub async fn cancel(&self, mode: CancelMode, grace: Duration) -> Result<()> {
        self.cancelled.store(true, Ordering::SeqCst);
        match mode {
            CancelMode::Forced => self.terminate_tree(),
            CancelMode::Graceful => {
                self.close_stdin().await?;
                let mut rx = self.exit_rx.clone();
                let exited = tokio::time::timeout(grace, async {
                    loop {
                        if rx.borrow().is_some() {
                            return;
                        }
                        if rx.changed().await.is_err() {
                            return;
                        }
                    }
                })
                .await
                .is_ok();
                if exited {
                    tracing::debug!(pid = self.pid, "process exited within the graceful window");
                    Ok(())
                } else {
                    tracing::debug!(
                        pid = self.pid,
                        grace_ms = grace.as_millis() as u64,
                        "process ignored the graceful request; terminating the tree"
                    );
                    self.terminate_tree()
                }
            }
        }
    }

    fn terminate_tree(&self) -> Result<()> {
        match &self.job {
            Some(job) => {
                job.terminate(1)?;
                Ok(())
            }
            None => {
                containment::terminate_process(self.pid)?;
                Ok(())
            }
        }
    }

    /// Block until the process exits, delivering every line to the sink
    /// first. Safe to call once; calling it again returns the same result
    /// immediately.
    pub async fn wait(&self) -> Result<ProcessExit> {
        let mut rx = self.exit_rx.clone();
        let exit_code = loop {
            let current = *rx.borrow();
            if current.is_some() {
                break current;
            }
            if rx.changed().await.is_err() {
                // The sender is dropped only after it has sent, but treat a
                // closed channel as "exited, code unknown" rather than
                // hanging the caller forever.
                break None;
            }
        };

        let readers = std::mem::take(&mut *self.readers.lock().await);
        for reader in readers {
            if tokio::time::timeout(OUTPUT_LINGER, reader).await.is_err() {
                tracing::warn!(
                    pid = self.pid,
                    "process output pipe still open after the process exited (likely inherited by a grandchild); no further lines will be read"
                );
            }
        }
        if let Some(watcher) = self.watcher.lock().await.take() {
            let _ = watcher.await;
        }

        Ok(ProcessExit {
            exit_code,
            cancelled: self.is_cancelled(),
        })
    }

    /// Non-blocking check for exit. `None` means still running.
    pub fn try_exit(&self) -> Option<ProcessExit> {
        let exit_code = *self.exit_rx.borrow();
        exit_code.map(|exit_code| ProcessExit {
            exit_code: Some(exit_code),
            cancelled: self.is_cancelled(),
        })
    }
}

impl Drop for SupervisedProcess {
    fn drop(&mut self) {
        // Fail-safe, not a normal path: if the handle is dropped while the
        // process is still running, kill the tree rather than let it keep
        // mutating a worktree with nobody watching. For the contained case
        // the job's own `Drop` already does this via KILL_ON_JOB_CLOSE; the
        // explicit call makes the intent local and covers the
        // no-job fallback path.
        if self.try_exit().is_none() && !self.is_cancelled() {
            tracing::debug!(
                pid = self.pid,
                label = %self.label,
                "supervised process handle dropped while running; terminating its tree"
            );
            let _ = self.terminate_tree();
        }
    }
}

/// Buffers every line, for [`ProcessSupervisor::capture`]. Private: the
/// public shape is [`CapturedOutput`], not a sink.
#[derive(Default)]
struct BufferSink(StdMutex<Vec<ProcessLine>>);

impl BufferSink {
    fn take_joined(&self) -> (String, String) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        for line in self.0.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
            match line.stream {
                ProcessStream::Stdout => stdout.push(line.text),
                ProcessStream::Stderr => stderr.push(line.text),
            }
        }
        (stdout.join("\n"), stderr.join("\n"))
    }
}

impl LineSink for BufferSink {
    fn line(&self, line: ProcessLine) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct VecSink(StdMutex<Vec<ProcessLine>>);

    impl VecSink {
        fn new() -> Arc<Self> {
            Arc::new(Self(StdMutex::new(Vec::new())))
        }

        fn texts(&self) -> Vec<String> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .map(|line| line.text.clone())
                .collect()
        }
    }

    impl LineSink for VecSink {
        fn line(&self, line: ProcessLine) {
            self.0.lock().unwrap().push(line);
        }
    }

    fn test_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nacc-process-test-{label}-{}",
            nacc_domain::ProjectId::new()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn stdout_lines_are_delivered_to_the_sink_and_the_exit_code_is_captured() {
        let sink = VecSink::new();
        let dir = std::env::temp_dir();
        let spec = ProcessSpec::new("cmd.exe", &dir)
            .args(["/C", "echo nacc-line-capture-check"])
            .label("line-capture");

        let process = ProcessSupervisor::new()
            .spawn(spec, sink.clone())
            .await
            .expect("cmd.exe must exist on Windows");
        let exit = process.wait().await.unwrap();

        assert!(exit.succeeded(), "echo must exit 0: {exit:?}");
        assert!(
            sink.texts()
                .iter()
                .any(|line| line.contains("nacc-line-capture-check")),
            "the process's stdout line must reach the sink: {:?}",
            sink.texts()
        );
    }

    #[tokio::test]
    async fn capture_returns_stdout_stderr_and_the_exit_code_together() {
        let spec = ProcessSpec::new("cmd.exe", std::env::temp_dir())
            .args(["/C", "echo captured-line && echo captured-error 1>&2"])
            .label("capture");
        let out = ProcessSupervisor::new().capture(spec).await.unwrap();
        assert!(out.succeeded(), "{out:?}");
        assert!(out.stdout.contains("captured-line"), "{:?}", out.stdout);
        assert!(out.stderr.contains("captured-error"), "{:?}", out.stderr);
    }

    #[tokio::test]
    async fn stderr_is_kept_separate_from_stdout() {
        let sink = VecSink::new();
        let spec = ProcessSpec::new("cmd.exe", std::env::temp_dir())
            .args(["/C", "echo to-stderr 1>&2"])
            .label("stream-separation");

        let process = ProcessSupervisor::new().spawn(spec, sink.clone()).await.unwrap();
        process.wait().await.unwrap();

        let stderr_lines: Vec<String> = sink
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.stream == ProcessStream::Stderr)
            .map(|line| line.text.clone())
            .collect();
        assert!(
            stderr_lines.iter().any(|line| line.contains("to-stderr")),
            "stderr must be attributed to the stderr stream: {stderr_lines:?}"
        );
    }

    /// Master plan S13.4's requirement, verified against the OS rather than
    /// against a return value: a two-level tree (`cmd.exe` -> Powershell ->
    /// Powershell) must be completely gone after a forced cancel.
    #[tokio::test]
    async fn forced_cancel_kills_the_entire_process_tree_including_grandchildren() {
        if !cfg!(windows) {
            return;
        }
        let dir = test_dir("tree");
        let grandchild_pid_file = dir.join("grandchild.pid");
        let child_script = dir.join("child.ps1");
        let parent_script = dir.join("parent.ps1");

        std::fs::write(
            &child_script,
            format!(
                "[IO.File]::WriteAllText('{}', \"$PID\")\nStart-Sleep -Seconds 120\n",
                grandchild_pid_file.display()
            ),
        )
        .unwrap();
        std::fs::write(
            &parent_script,
            format!(
                "Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File', ('\"' + '{}' + '\"'))\nStart-Sleep -Seconds 120\n",
                child_script.display()
            ),
        )
        .unwrap();

        let spec = ProcessSpec::new("cmd.exe", &dir)
            .args([
                "/C",
                "powershell.exe",
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(parent_script.to_string_lossy().into_owned())
            .label("tree-cancel");

        let process = ProcessSupervisor::new()
            .spawn(spec, Arc::new(DiscardLines))
            .await
            .unwrap();
        assert!(
            process.is_contained(),
            "the tree test is only meaningful with real job-object containment"
        );

        let grandchild = wait_for_pid_file(&grandchild_pid_file, Duration::from_secs(60))
            .await
            .unwrap_or_else(|| panic!("grandchild never wrote {}", grandchild_pid_file.display()));
        assert!(
            containment::process_alive(grandchild),
            "the grandchild must be running before we cancel"
        );
        let direct_child = process.pid();

        process
            .cancel(CancelMode::Forced, Duration::from_secs(1))
            .await
            .unwrap();
        let exit = process.wait().await.unwrap();
        assert!(exit.cancelled, "the exit must be reported as a cancellation");

        wait_until_dead(grandchild, Duration::from_secs(20)).await;
        assert!(
            !containment::process_alive(grandchild),
            "grandchild {grandchild} survived a forced cancellation — the job object did not contain the tree"
        );
        assert!(
            !containment::process_alive(direct_child),
            "direct child {direct_child} survived a forced cancellation"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The graceful path must actually be graceful: a process that ends on
    /// end-of-stdin exits on its own, before the grace period expires, and
    /// is still reported as cancelled.
    #[tokio::test]
    async fn graceful_cancel_lets_a_stdin_terminated_process_exit_on_its_own() {
        if !cfg!(windows) {
            return;
        }
        // `more` copies stdin to stdout and exits at end-of-input.
        let spec = ProcessSpec::new("cmd.exe", std::env::temp_dir())
            .args(["/C", "more"])
            .label("graceful-cancel");
        let process = ProcessSupervisor::new()
            .spawn(spec, Arc::new(DiscardLines))
            .await
            .unwrap();
        process.write_stdin(b"hello\n").await.unwrap();

        let started = std::time::Instant::now();
        process
            .cancel(CancelMode::Graceful, Duration::from_secs(20))
            .await
            .unwrap();
        let exit = process.wait().await.unwrap();

        assert!(exit.cancelled);
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "a graceful cancel must not wait out the whole grace period when the process exits on EOF (took {:?})",
            started.elapsed()
        );
    }

    /// Dropping the handle without cancelling must not leave the tree
    /// running — this is the anti-orphan guarantee, not a convenience.
    #[tokio::test]
    async fn dropping_the_handle_terminates_a_running_process() {
        if !cfg!(windows) {
            return;
        }
        let dir = test_dir("drop");
        let pid_file = dir.join("dropped.pid");
        let script = dir.join("sleep.ps1");
        std::fs::write(
            &script,
            format!(
                "[IO.File]::WriteAllText('{}', \"$PID\")\nStart-Sleep -Seconds 120\n",
                pid_file.display()
            ),
        )
        .unwrap();

        let spec = ProcessSpec::new("powershell.exe", &dir)
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(script.to_string_lossy().into_owned())
            .label("drop-handle");
        let process = ProcessSupervisor::new()
            .spawn(spec, Arc::new(DiscardLines))
            .await
            .unwrap();
        let pid = wait_for_pid_file(&pid_file, Duration::from_secs(60))
            .await
            .unwrap_or_else(|| panic!("child never wrote {}", pid_file.display()));

        drop(process);

        wait_until_dead(pid, Duration::from_secs(20)).await;
        assert!(
            !containment::process_alive(pid),
            "dropping the supervisor handle must terminate the process, not orphan it"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    async fn wait_for_pid_file(path: &std::path::Path, timeout: Duration) -> Option<u32> {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if let Ok(contents) = std::fs::read_to_string(path) {
                if let Ok(pid) = contents.trim().parse::<u32>() {
                    return Some(pid);
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        None
    }

    async fn wait_until_dead(pid: u32, timeout: Duration) {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if !containment::process_alive(pid) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
}

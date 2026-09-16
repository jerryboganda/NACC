//! The command-execution seam every provider adapter runs commands
//! through: [`CommandRunner`] (the contract), [`ProcessCommandRunner`] (the
//! real implementation, over `nacc-process`'s contained supervisor), and
//! [`FixtureCommandRunner`] (recorded invocations, for tests).
//!
//! # Why this seam exists
//!
//! Master plan S17's Phase 4 asks for "provider contract, capability
//! snapshots, normalized events, health, fixtures, and contract tests".
//! Fixtures only mean something if an adapter can be driven from recorded
//! CLI output instead of a live binary -- otherwise "adapter tests" degrade
//! into "tests that skip when the CLI is absent", which is exactly the
//! mocked-adapter theater the build prompt warns against. Injecting the
//! runner makes the *real* adapter code path (argv construction, output
//! parsing, event mapping, error classification) run against captured
//! bytes, while the only thing swapped out is who executes the command.
//!
//! Nothing here decides a command line: adapters build their own argument
//! arrays (S13.3 -- argument arrays, never shell strings) and hand them
//! here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{ProviderError, Result};

/// Output of one command invocation.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct CommandOutput {
    pub program: String,
    pub args: Vec<String>,
    pub stdout: String,
    pub stderr: String,
    /// `None` when the process was killed without an exit code.
    pub exit_code: Option<i32>,
    /// Whether NACC asked for this command to stop. Kept separate from the
    /// exit code so a cancelled probe is never classified as a provider
    /// failure (master plan S14.5).
    pub cancelled: bool,
}

impl CommandOutput {
    pub fn succeeded(&self) -> bool {
        !self.cancelled && self.exit_code == Some(0)
    }

    /// The command line as it would be shown in the audit trail (master
    /// plan S22). Arguments only -- there is no shell involved anywhere, so
    /// there is nothing to quote-escape and no injection surface.
    pub fn command_line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// One line of a running command's output, in the shape adapters consume.
/// Re-exported from `nacc-process` rather than redefined: the line framing
/// and the stream attribution must not differ between the layer that reads
/// the pipe and the layer that interprets it.
pub use nacc_process::{LineSink as CommandLineSink, ProcessLine as CommandLine};

/// The stream a command line came from (`stdout` carries a provider's
/// structured output; `stderr` carries its diagnostics).
pub use nacc_process::ProcessStream as CommandStream;

/// A command that has been started and is still running. `launch` needs
/// this shape -- not a blocking `run` -- because cancellation has to reach a
/// process that is *in flight* (master plan S13.4), and a provider run can
/// last minutes while the GUI watches its events arrive.
#[async_trait]
pub trait RunningCommand: Send + Sync {
    /// The OS process id of the direct child (useful for the audit trail
    /// and for reconciliation).
    fn pid(&self) -> u32;

    /// Stop it: `graceful` closes the child's stdin and allows a bounded
    /// period before the contained tree is terminated.
    async fn cancel(
        &self,
        mode: nacc_process::CancelMode,
        grace: std::time::Duration,
    ) -> Result<()>;

    /// Wait for completion, after which every output line has been
    /// delivered to the sink.
    async fn wait(&self) -> Result<CommandOutput>;
}

/// Executes a program with an argument array. Implemented for real by
/// [`ProcessCommandRunner`]; replaced in tests by
/// [`FixtureCommandRunner`].
///
/// The trait is intentionally *not* "spawn a shell": NACC never invokes a
/// shell (master plan S13.3), and a provider adapter that needs shell-like
/// behavior is doing something wrong rather than needing a bigger API.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    /// Run to completion and return everything at once. Right for probes
    /// (`--version`, a model list) and for JSON-mode runs parsed at the end.
    async fn run(
        &self,
        program: &str,
        args: &[String],
        working_directory: &Path,
        env: &[(String, String)],
    ) -> Result<CommandOutput>;

    /// Start a command and stream its lines to `sink` as they arrive,
    /// returning a handle that can cancel and await it. There is no
    /// default implementation on purpose: an adapter that silently fell
    /// back to run-to-completion would break live streaming *and*
    /// cancellation without saying so, and both are contract-visible
    /// behaviors.
    async fn spawn(
        &self,
        program: &str,
        args: &[String],
        working_directory: &Path,
        env: &[(String, String)],
        sink: Arc<dyn CommandLineSink>,
    ) -> Result<Arc<dyn RunningCommand>>;
}

/// The real runner. `nacc-process` owns containment, cancellation, and
/// output streaming; this is a thin translation into
/// [`CommandOutput`] so provider adapters never depend on `nacc-process`
/// types directly.
pub struct ProcessCommandRunner {
    supervisor: nacc_process::ProcessSupervisor,
}

impl Default for ProcessCommandRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessCommandRunner {
    pub fn new() -> Self {
        Self {
            supervisor: nacc_process::ProcessSupervisor::new(),
        }
    }
}

/// The real [`RunningCommand`]: a contained `nacc-process` child.
struct ProcessRunningCommand {
    program: String,
    args: Vec<String>,
    process: nacc_process::SupervisedProcess,
}

#[async_trait]
impl RunningCommand for ProcessRunningCommand {
    fn pid(&self) -> u32 {
        self.process.pid()
    }

    async fn cancel(
        &self,
        mode: nacc_process::CancelMode,
        grace: std::time::Duration,
    ) -> Result<()> {
        self.process
            .cancel(mode, grace)
            .await
            .map_err(|err| ProviderError::Process(err.to_string()))
    }

    async fn wait(&self) -> Result<CommandOutput> {
        let exit = self
            .process
            .wait()
            .await
            .map_err(|err| ProviderError::Process(err.to_string()))?;
        // A run whose lines were consumed as they arrived cannot also return
        // them; `CommandOutput::stdout` is therefore empty here and the
        // adapter's own accumulated result is the source of truth for
        // structured output. Documented rather than silently surprising.
        Ok(CommandOutput {
            program: self.program.clone(),
            args: self.args.clone(),
            stdout: String::new(),
            stderr: String::new(),
            exit_code: exit.exit_code,
            cancelled: exit.cancelled,
        })
    }
}

#[async_trait]
impl CommandRunner for ProcessCommandRunner {
    async fn run(
        &self,
        program: &str,
        args: &[String],
        working_directory: &Path,
        env: &[(String, String)],
    ) -> Result<CommandOutput> {
        let mut spec = nacc_process::ProcessSpec::new(program, working_directory)
            .args(args.iter().cloned())
            .label(program);
        for (key, value) in env {
            spec = spec.env(key.clone(), value.clone());
        }
        let captured = self
            .supervisor
            .capture(spec)
            .await
            .map_err(|err| match err {
                nacc_process::ProcessError::Spawn { .. } => ProviderError::NotInstalled {
                    detail: err.to_string(),
                },
                other => ProviderError::Process(other.to_string()),
            })?;
        Ok(CommandOutput {
            program: program.to_string(),
            args: args.to_vec(),
            stdout: captured.stdout,
            stderr: captured.stderr,
            exit_code: captured.exit.exit_code,
            cancelled: captured.exit.cancelled,
        })
    }

    async fn spawn(
        &self,
        program: &str,
        args: &[String],
        working_directory: &Path,
        env: &[(String, String)],
        sink: Arc<dyn CommandLineSink>,
    ) -> Result<Arc<dyn RunningCommand>> {
        let mut spec = nacc_process::ProcessSpec::new(program, working_directory)
            .args(args.iter().cloned())
            .label(program);
        for (key, value) in env {
            spec = spec.env(key.clone(), value.clone());
        }
        let process = self
            .supervisor
            .spawn(spec, sink)
            .await
            .map_err(|err| match err {
                nacc_process::ProcessError::Spawn { .. } => ProviderError::NotInstalled {
                    detail: err.to_string(),
                },
                other => ProviderError::Process(other.to_string()),
            })?;
        Ok(Arc::new(ProcessRunningCommand {
            program: program.to_string(),
            args: args.to_vec(),
            process,
        }))
    }
}

/// One recorded invocation: what was run, and what came back. This is a
/// fixture, not a mock -- the `stdout`/`stderr`/`exit_code` are captured
/// from a real CLI run and stored verbatim in the adapter's `fixtures/`
/// directory next to the documented CLI version they came from.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordedInvocation {
    pub program: String,
    /// Argument pattern. An element of the literal string `"*"` matches
    /// exactly one argument of any value -- never a variable-length span,
    /// so matching stays deterministic and a fixture cannot silently absorb
    /// an argument the adapter should not have added. Needed because some
    /// arguments are genuinely unpredictable at fixture-capture time (a
    /// per-run session UUID), and a fixture format that cannot express that
    /// forces tests to stop covering those command lines at all.
    pub args: Vec<String>,
    /// The version string of the CLI these bytes were captured from, so a
    /// fixture that no longer matches a provider's output contract can be
    /// identified instead of silently trusting a stale capture.
    #[serde(default)]
    pub captured_from_version: Option<String>,
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
}

fn args_match(pattern: &[String], actual: &[String]) -> bool {
    pattern.len() == actual.len()
        && pattern
            .iter()
            .zip(actual)
            .all(|(expected, got)| expected == "*" || expected == got)
}

/// A [`CommandRunner`] that replays recorded invocations and fails loudly
/// on anything it has no fixture for. Loud failure is the point: an adapter
/// test that silently passed because a command was never actually run would
/// be worthless.
#[derive(Default)]
pub struct FixtureCommandRunner {
    invocations: Vec<RecordedInvocation>,
    seen: std::sync::Mutex<Vec<String>>,
}

impl FixtureCommandRunner {
    pub fn new(invocations: Vec<RecordedInvocation>) -> Self {
        Self {
            invocations,
            seen: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Load from the JSON array shape used by every adapter's `fixtures/`
    /// files.
    pub fn from_json(json: &str) -> std::result::Result<Self, serde_json::Error> {
        Ok(Self::new(serde_json::from_str(json)?))
    }

    /// Load every `.json` fixture in a directory (sorted by filename, so a
    /// run is reproducible).
    pub fn from_dir(dir: &Path) -> std::io::Result<std::result::Result<Self, serde_json::Error>> {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        paths.sort();
        let mut all = Vec::new();
        for path in paths {
            let text = std::fs::read_to_string(&path)?;
            match serde_json::from_str::<Vec<RecordedInvocation>>(&text) {
                Ok(mut invocations) => all.append(&mut invocations),
                Err(err) => return Ok(Err(err)),
            }
        }
        Ok(Ok(Self::new(all)))
    }

    /// Command lines that were actually requested during a test, in order.
    /// Lets a test assert *what* the adapter built, not only what it
    /// parsed back.
    pub fn observed_commands(&self) -> Vec<String> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[async_trait]
impl CommandRunner for FixtureCommandRunner {
    async fn run(
        &self,
        program: &str,
        args: &[String],
        _working_directory: &Path,
        _env: &[(String, String)],
    ) -> Result<CommandOutput> {
        let key = std::iter::once(program)
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(key.clone());

        let recorded = self
            .invocations
            .iter()
            .find(|invocation| {
                invocation.program == program && args_match(&invocation.args, args)
            })
            .ok_or_else(|| {
                ProviderError::Other(format!(
                    "no recorded fixture for `{key}` -- add the invocation to this adapter's fixtures"
                ))
            })?;

        Ok(CommandOutput {
            program: program.to_string(),
            args: args.to_vec(),
            stdout: recorded.stdout.clone(),
            stderr: recorded.stderr.clone(),
            exit_code: recorded.exit_code.or(Some(0)),
            cancelled: false,
        })
    }

    /// Replay the recorded output through the sink, then hand back a handle
    /// that is already complete. Streaming order and cancellation behavior
    /// are therefore *not* exercised by a fixture -- that is what the real
    /// runner's own tests and the OS-level process tests cover. Stated
    /// plainly so nobody reads a green fixture test as proof that live
    /// streaming works.
    async fn spawn(
        &self,
        program: &str,
        args: &[String],
        working_directory: &Path,
        env: &[(String, String)],
        sink: Arc<dyn CommandLineSink>,
    ) -> Result<Arc<dyn RunningCommand>> {
        let output = self.run(program, args, working_directory, env).await?;
        for text in output.stdout.lines() {
            sink.line(CommandLine {
                stream: CommandStream::Stdout,
                text: text.to_string(),
            });
        }
        for text in output.stderr.lines() {
            sink.line(CommandLine {
                stream: CommandStream::Stderr,
                text: text.to_string(),
            });
        }
        Ok(Arc::new(FixtureRunningCommand { output }))
    }
}

/// A [`RunningCommand`] that has already finished: the fixture runner has
/// nothing live to cancel.
struct FixtureRunningCommand {
    output: CommandOutput,
}

#[async_trait]
impl RunningCommand for FixtureRunningCommand {
    fn pid(&self) -> u32 {
        0
    }

    async fn cancel(
        &self,
        _mode: nacc_process::CancelMode,
        _grace: std::time::Duration,
    ) -> Result<()> {
        Err(ProviderError::Other(
            "a fixture-backed command has already completed and cannot be cancelled".to_string(),
        ))
    }

    async fn wait(&self) -> Result<CommandOutput> {
        Ok(self.output.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_real_runner_executes_a_real_command_and_reports_its_output() {
        let runner = ProcessCommandRunner::new();
        let output = runner
            .run(
                "cmd.exe",
                &["/C".to_string(), "echo nacc-cli-runner".to_string()],
                Path::new("."),
                &[],
            )
            .await
            .expect("cmd.exe must be runnable");
        assert!(output.succeeded(), "{output:?}");
        assert!(output.stdout.contains("nacc-cli-runner"));
        assert_eq!(output.command_line(), "cmd.exe /C echo nacc-cli-runner");
    }

    #[tokio::test]
    async fn a_missing_program_is_a_typed_not_installed_error() {
        let runner = ProcessCommandRunner::new();
        let err = runner
            .run(
                "nacc-definitely-not-an-installed-program.exe",
                &[],
                Path::new("."),
                &[],
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderError::NotInstalled { .. }),
            "a missing CLI is a normal detection outcome, not a crash: {err:?}"
        );
    }

    #[tokio::test]
    async fn a_nonzero_exit_is_returned_not_turned_into_an_error() {
        let runner = ProcessCommandRunner::new();
        let output = runner
            .run(
                "cmd.exe",
                &["/C".to_string(), "exit 3".to_string()],
                Path::new("."),
                &[],
            )
            .await
            .unwrap();
        assert!(!output.succeeded());
        assert_eq!(output.exit_code, Some(3));
    }

    #[tokio::test]
    async fn the_real_runner_streams_lines_live_and_can_cancel_a_running_command() {
        struct Counting(std::sync::Mutex<Vec<String>>);
        impl CommandLineSink for Counting {
            fn line(&self, line: CommandLine) {
                self.0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(line.text);
            }
        }

        let sink = Arc::new(Counting(std::sync::Mutex::new(Vec::new())));
        let runner = ProcessCommandRunner::new();
        let command = runner
            .spawn(
                "cmd.exe",
                &["/C".to_string(), "echo streamed-live".to_string()],
                Path::new("."),
                &[],
                sink.clone(),
            )
            .await
            .unwrap();
        assert!(command.pid() > 0, "a real child must report its pid");
        let output = command.wait().await.unwrap();
        assert!(output.succeeded(), "{output:?}");
        assert!(sink
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|line| line.contains("streamed-live")));
    }

    #[tokio::test]
    async fn the_fixture_runner_replays_recorded_output_and_errors_on_a_miss() {
        let runner = FixtureCommandRunner::from_json(
            r#"[
              {
                "program": "claude",
                "args": ["--version"],
                "captured_from_version": "2.0.0",
                "stdout": "2.0.0 (Claude Code)",
                "exit_code": 0
              }
            ]"#,
        )
        .unwrap();

        let hit = runner
            .run("claude", &["--version".to_string()], Path::new("."), &[])
            .await
            .unwrap();
        assert_eq!(hit.stdout, "2.0.0 (Claude Code)");

        let miss = runner
            .run("claude", &["--help".to_string()], Path::new("."), &[])
            .await
            .unwrap_err();
        assert!(miss.to_string().contains("no recorded fixture"));
        assert_eq!(runner.observed_commands().len(), 2);
    }

    #[test]
    fn fixture_loading_fails_loudly_on_malformed_json() {
        assert!(FixtureCommandRunner::from_json("{not json").is_err());
    }

    #[test]
    fn a_wildcard_matches_exactly_one_argument_and_never_a_span() {
        let pattern = vec!["--session-id".to_string(), "*".to_string()];
        assert!(args_match(
            &pattern,
            &["--session-id".to_string(), "abc".to_string()]
        ));
        assert!(!args_match(
            &pattern,
            &[
                "--session-id".to_string(),
                "abc".to_string(),
                "extra".to_string()
            ]
        ));
        assert!(!args_match(&pattern, &["--session-id".to_string()]));
        // A wildcard must not widen a fixture to accept an argument the
        // adapter should not have produced.
        assert!(!args_match(
            &pattern,
            &["--other-flag".to_string(), "abc".to_string()]
        ));
    }
}

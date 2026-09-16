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

/// Executes a program with an argument array. Implemented for real by
/// [`ProcessCommandRunner`]; replaced in tests by
/// [`FixtureCommandRunner`].
///
/// The trait is intentionally *not* "spawn a shell": NACC never invokes a
/// shell (master plan S13.3), and a provider adapter that needs shell-like
/// behavior is doing something wrong rather than needing a bigger API.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(
        &self,
        program: &str,
        args: &[String],
        working_directory: &Path,
        env: &[(String, String)],
    ) -> Result<CommandOutput>;
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
}

/// One recorded invocation: what was run, and what came back. This is a
/// fixture, not a mock -- the `stdout`/`stderr`/`exit_code` are captured
/// from a real CLI run and stored verbatim in the adapter's `fixtures/`
/// directory next to the documented CLI version they came from.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordedInvocation {
    pub program: String,
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
            .find(|invocation| invocation.program == program && invocation.args == args)
            .ok_or_else(|| {
                ProviderError::Other(format!(
                "no recorded fixture for `{key}` -- add the invocation to this adapter's fixtures", 
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
}

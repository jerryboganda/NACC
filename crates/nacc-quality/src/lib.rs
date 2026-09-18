//! Deterministic quality gates (master plan S20, S27.19): a gate is a
//! declared command whose *exit code*, not any model's claim, decides pass
//! or fail. The runner executes the command in a worktree with a timeout
//! and records structured evidence -- exact command, exact exit code, the
//! bounded tail of combined output -- so review and repair flows argue from
//! the same facts the gate did.
//!
//! Phase 9 scope: this crate is the runner and evidence record. Workflow nodes
//! carry declarative quality-gate specs in `nacc-domain`, and the orchestrator
//! executes those declarations after provider success, persists the evidence,
//! and blocks completion on required failures. Review dispositions and repair
//! workflow policy remain separate lifecycle concerns.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nacc_domain::QualityGateSpec;
use nacc_policy::{Decision, PolicyEngine};
use nacc_process::{CancelMode, LineSink, ProcessLine, ProcessSpec, ProcessSupervisor};

/// One declared gate command. `argv` is the exact argument list -- never a
/// shell string, so nothing here is interpretable by a shell.
#[derive(Clone, Debug, PartialEq)]
pub struct GateCommand {
    /// Stable name the evidence and the handoff cross-check refer to.
    pub name: String,
    pub argv: Vec<String>,
    pub timeout_secs: u64,
}

impl From<&QualityGateSpec> for GateCommand {
    fn from(spec: &QualityGateSpec) -> Self {
        Self {
            name: spec.name.clone(),
            argv: spec.argv.clone(),
            timeout_secs: u64::from(spec.timeout_secs),
        }
    }
}

/// The bounded tail of combined stdout+stderr, so a huge build log cannot
/// bloat the durable record. The head is marked as trimmed.
const MAX_TAIL_CHARS: usize = 4_000;
const PROCESS_SETTLE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct GateOutputSink(Mutex<Vec<ProcessLine>>);

impl GateOutputSink {
    fn combined(&self) -> String {
        let lines = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl LineSink for GateOutputSink {
    fn line(&self, line: ProcessLine) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(line);
    }
}

fn redact_untrusted_text(text: &str) -> String {
    nacc_secrets::redact(text, &[]).0
}

fn bounded_tail(text: &str) -> String {
    let char_count = text.chars().count();
    if char_count <= MAX_TAIL_CHARS {
        return text.to_string();
    }

    let start = text
        .char_indices()
        .rev()
        .nth(MAX_TAIL_CHARS - 1)
        .map(|(index, _)| index)
        .unwrap_or(0);
    format!("[…trimmed…]{}", &text[start..])
}

#[derive(Clone, Debug, PartialEq)]
pub struct QualityEvidence {
    pub gate: String,
    pub command: String,
    pub passed: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub log_tail: String,
}

/// Run one gate in `workdir` and produce its evidence. The process is not a
/// shell: `argv[0]` is the program, the rest are arguments, inherited env.
pub async fn run_gate(workdir: &Path, gate: &GateCommand) -> QualityEvidence {
    let policy = PolicyEngine::baseline();
    run_gate_with_policy(workdir, gate, &policy).await
}

/// Run one gate after applying an explicit policy. This seam keeps policy
/// decisions testable and lets the application add resource-specific rules
/// without weakening the baseline guardrails used by [`run_gate`].
pub async fn run_gate_with_policy(
    workdir: &Path,
    gate: &GateCommand,
    policy: &PolicyEngine,
) -> QualityEvidence {
    let command_display = redact_untrusted_text(&gate.argv.join(" "));
    let started = Instant::now();

    if gate.argv.is_empty() {
        return QualityEvidence {
            gate: gate.name.clone(),
            command: command_display,
            passed: false,
            exit_code: None,
            timed_out: false,
            duration_ms: 0,
            log_tail: "gate command is empty".to_string(),
        };
    }

    if let Decision::Deny { reason } = policy.check_command(&gate.argv, 0) {
        return QualityEvidence {
            gate: gate.name.clone(),
            command: command_display,
            passed: false,
            exit_code: None,
            timed_out: false,
            duration_ms: started.elapsed().as_millis() as u64,
            log_tail: redact_untrusted_text(&format!("gate command denied by policy: {reason}")),
        };
    }

    let spec = ProcessSpec::new(gate.argv[0].clone(), workdir)
        .args(gate.argv[1..].iter().cloned())
        .label(format!("quality gate: {}", gate.name));
    let sink = Arc::new(GateOutputSink::default());
    let supervisor = ProcessSupervisor::new();
    let process = match supervisor.spawn(spec, sink.clone()).await {
        Ok(process) => process,
        Err(spawn_error) => {
            let duration_ms = started.elapsed().as_millis() as u64;
            return QualityEvidence {
                gate: gate.name.clone(),
                command: command_display,
                passed: false,
                exit_code: None,
                timed_out: false,
                duration_ms,
                log_tail: redact_untrusted_text(&format!(
                    "could not start the gate command: {spawn_error}"
                )),
            };
        }
    };
    // Quality gates are non-interactive. Preserve the previous runner's
    // `Stdio::null()` semantics by delivering EOF immediately rather than
    // leaving the supervisor's interactive stdin pipe open until timeout.
    if let Err(close_error) = process.close_stdin().await {
        let _ = process.cancel(CancelMode::Forced, Duration::ZERO).await;
        let _ = tokio::time::timeout(PROCESS_SETTLE_TIMEOUT, process.wait()).await;
        return QualityEvidence {
            gate: gate.name.clone(),
            command: command_display,
            passed: false,
            exit_code: None,
            timed_out: false,
            duration_ms: started.elapsed().as_millis() as u64,
            log_tail: redact_untrusted_text(&format!(
                "could not close gate stdin before execution: {close_error}"
            )),
        };
    }

    let outcome = tokio::time::timeout(
        Duration::from_secs(gate.timeout_secs.max(1)),
        process.wait(),
    )
    .await;

    match outcome {
        Err(_elapsed) => {
            let cancel_result = process.cancel(CancelMode::Forced, Duration::ZERO).await;
            let settle_result = tokio::time::timeout(PROCESS_SETTLE_TIMEOUT, process.wait()).await;
            let duration_ms = started.elapsed().as_millis() as u64;
            let mut detail = match (&cancel_result, &settle_result) {
                (Ok(()), Ok(Ok(_))) => format!(
                    "gate exceeded {}s and its process tree was terminated",
                    gate.timeout_secs
                ),
                (Err(error), _) => format!(
                    "gate exceeded {}s; process-tree termination failed: {error}",
                    gate.timeout_secs
                ),
                (Ok(()), Ok(Err(error))) => format!(
                    "gate exceeded {}s; process tree was terminated but wait failed: {error}",
                    gate.timeout_secs
                ),
                (Ok(()), Err(_)) => format!(
                    "gate exceeded {}s; process-tree termination did not settle within {}s",
                    gate.timeout_secs,
                    PROCESS_SETTLE_TIMEOUT.as_secs()
                ),
            };
            let output = sink.combined();
            if !output.is_empty() {
                detail.push('\n');
                detail.push_str(&output);
            }
            QualityEvidence {
                gate: gate.name.clone(),
                command: command_display,
                passed: false,
                exit_code: None,
                timed_out: true,
                duration_ms,
                log_tail: bounded_tail(&redact_untrusted_text(&detail)),
            }
        }
        Ok(Err(wait_error)) => QualityEvidence {
            gate: gate.name.clone(),
            command: command_display,
            passed: false,
            exit_code: None,
            timed_out: false,
            duration_ms: started.elapsed().as_millis() as u64,
            log_tail: redact_untrusted_text(&format!("gate process wait failed: {wait_error}")),
        },
        Ok(Ok(exit)) => {
            let log_tail = bounded_tail(&redact_untrusted_text(&sink.combined()));
            QualityEvidence {
                gate: gate.name.clone(),
                command: command_display,
                passed: exit.succeeded(),
                exit_code: exit.exit_code,
                timed_out: false,
                duration_ms: started.elapsed().as_millis() as u64,
                log_tail,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_declaration_maps_to_runner_command_without_changing_semantics() {
        let spec = QualityGateSpec {
            name: "workspace-tests".into(),
            argv: vec!["cargo".into(), "test".into(), "--workspace".into()],
            timeout_secs: 900,
            required: false,
        };
        let command = GateCommand::from(&spec);
        assert_eq!(command.name, spec.name);
        assert_eq!(command.argv, spec.argv);
        assert_eq!(command.timeout_secs, 900);
    }

    fn pass_gate() -> GateCommand {
        GateCommand {
            name: "always-passes".to_string(),
            argv: vec!["cmd".into(), "/c".into(), "exit 0".into()],
            timeout_secs: 30,
        }
    }

    fn fail_gate() -> GateCommand {
        GateCommand {
            name: "always-fails".to_string(),
            argv: vec![
                "cmd".into(),
                "/c".into(),
                "echo gate-output & exit 3".into(),
            ],
            timeout_secs: 30,
        }
    }

    #[tokio::test]
    async fn a_zero_exit_code_passes_and_records_the_exact_command() {
        let evidence = run_gate(Path::new("."), &pass_gate()).await;
        assert!(evidence.passed);
        assert_eq!(evidence.exit_code, Some(0));
        assert!(!evidence.timed_out);
        assert_eq!(evidence.command, "cmd /c exit 0");
    }

    #[tokio::test]
    async fn a_gate_receives_eof_on_stdin_instead_of_waiting_for_input() {
        if !cfg!(windows) {
            return;
        }
        let gate = GateCommand {
            name: "stdin-eof".to_string(),
            argv: vec!["cmd.exe".into(), "/C".into(), "more >nul".into()],
            timeout_secs: 2,
        };
        let evidence = run_gate(Path::new("."), &gate).await;
        assert!(evidence.passed, "{}", evidence.log_tail);
        assert!(!evidence.timed_out);
    }

    #[tokio::test]
    async fn a_nonzero_exit_code_fails_and_captures_the_log_tail() {
        let evidence = run_gate(Path::new("."), &fail_gate()).await;
        assert!(!evidence.passed);
        assert_eq!(evidence.exit_code, Some(3));
        assert!(
            evidence.log_tail.contains("gate-output"),
            "output is the evidence"
        );
    }

    #[tokio::test]
    async fn a_hanging_gate_times_out_with_named_evidence() {
        let gate = GateCommand {
            name: "hangs".to_string(),
            argv: vec![
                "cmd".into(),
                "/c".into(),
                "ping -n 30 127.0.0.1 >nul".into(),
            ],
            timeout_secs: 1,
        };
        let started = Instant::now();
        let evidence = run_gate(Path::new("."), &gate).await;
        let elapsed = started.elapsed();
        assert!(!evidence.passed);
        assert!(evidence.timed_out);
        assert_eq!(evidence.exit_code, None);
        assert!(
            elapsed < Duration::from_secs(5),
            "timed-out gate must terminate its process tree promptly; elapsed={elapsed:?}"
        );
    }

    #[tokio::test]
    async fn a_timed_out_gate_kills_its_descendant_processes() {
        if !cfg!(windows) {
            return;
        }

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nacc-quality-tree-{unique}"));
        std::fs::create_dir_all(&dir).expect("create quality-gate test directory");
        let pid_file = dir.join("grandchild.pid");
        let child_script = dir.join("child.ps1");
        let parent_script = dir.join("parent.ps1");

        let escaped_pid_file = pid_file.to_string_lossy().replace('\'', "''");
        let escaped_child_script = child_script.to_string_lossy().replace('\'', "''");
        std::fs::write(
            &child_script,
            format!(
                "[IO.File]::WriteAllText('{escaped_pid_file}', \"$PID\")\nStart-Sleep -Seconds 30\n"
            ),
        )
        .expect("write descendant script");
        std::fs::write(
            &parent_script,
            format!(
                "Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File', ('\"' + '{escaped_child_script}' + '\"'))\nStart-Sleep -Seconds 30\n"
            ),
        )
        .expect("write parent script");

        let gate = GateCommand {
            name: "tree-timeout".to_string(),
            argv: vec![
                "cmd.exe".into(),
                "/C".into(),
                "powershell.exe".into(),
                "-NoProfile".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                parent_script.to_string_lossy().into_owned(),
            ],
            timeout_secs: 3,
        };

        let evidence = run_gate(&dir, &gate).await;
        assert!(evidence.timed_out, "gate should reach its timeout");
        let pid: u32 = std::fs::read_to_string(&pid_file)
            .unwrap_or_else(|error| panic!("grandchild did not publish its PID: {error}"))
            .trim()
            .parse()
            .expect("grandchild PID is numeric");
        assert!(
            !nacc_process::process_alive(pid),
            "grandchild {pid} survived quality-gate timeout"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_missing_binary_is_failure_evidence_not_a_panic() {
        let gate = GateCommand {
            name: "no-such-tool".to_string(),
            argv: vec!["definitely-not-a-real-binary-xyz".into()],
            timeout_secs: 5,
        };
        let evidence = run_gate(Path::new("."), &gate).await;
        assert!(!evidence.passed);
        assert!(evidence.log_tail.contains("could not start"));
    }

    #[tokio::test]
    async fn an_empty_gate_is_failure_evidence_not_a_panic() {
        let gate = GateCommand {
            name: "empty".to_string(),
            argv: vec![],
            timeout_secs: 5,
        };
        let evidence = run_gate(Path::new("."), &gate).await;
        assert!(!evidence.passed);
        assert_eq!(evidence.exit_code, None);
        assert!(evidence.log_tail.contains("empty"));
    }

    #[tokio::test]
    async fn baseline_policy_blocks_destructive_commands_before_spawn() {
        let gate = GateCommand {
            name: "blocked".to_string(),
            argv: vec!["git".into(), "push".into(), "--force".into()],
            timeout_secs: 5,
        };
        let evidence = run_gate(Path::new("."), &gate).await;
        assert!(!evidence.passed);
        assert_eq!(evidence.exit_code, None);
        assert!(evidence.log_tail.contains("denied by policy"));
        assert!(evidence.log_tail.contains("push --force"));
    }

    #[tokio::test]
    async fn command_and_output_are_redacted_before_becoming_evidence() {
        let token = "ghp_0123456789abcdefghijklmnopqrstuvwxyz";
        let gate = GateCommand {
            name: "redaction".to_string(),
            argv: vec!["cmd".into(), "/c".into(), format!("echo {token}")],
            timeout_secs: 5,
        };
        let evidence = run_gate(Path::new("."), &gate).await;
        assert!(evidence.passed);
        assert!(!evidence.command.contains(token));
        assert!(!evidence.log_tail.contains(token));
        assert!(evidence.command.contains("[REDACTED:ghp-token]"));
        assert!(evidence.log_tail.contains("[REDACTED:ghp-token]"));
    }

    #[test]
    fn bounded_tail_never_splits_multibyte_text() {
        let input = format!("prefix{}", "✓".repeat(MAX_TAIL_CHARS + 10));
        let tail = bounded_tail(&input);
        assert!(tail.starts_with("[…trimmed…]"));
        assert_eq!(tail.matches('✓').count(), MAX_TAIL_CHARS);
    }
}

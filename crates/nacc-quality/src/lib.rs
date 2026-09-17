//! Deterministic quality gates (master plan S20, S27.19): a gate is a
//! declared command whose *exit code*, not any model's claim, decides pass
//! or fail. The runner executes the command in a worktree with a timeout
//! and records structured evidence -- exact command, exact exit code, the
//! bounded tail of combined output -- so review and repair flows argue from
//! the same facts the gate did.
//!
//! Phase 9 scope: this crate is the runner and evidence record. Wiring gates
//! into workflow nodes (a `quality_gate` node kind) lands with the engine
//! integration in a later slice; the evidence shape here is what that
//! integration will persist.

use std::path::Path;
use std::time::{Duration, Instant};

/// One declared gate command. `argv` is the exact argument list -- never a
/// shell string, so nothing here is interpretable by a shell.
#[derive(Clone, Debug, PartialEq)]
pub struct GateCommand {
    /// Stable name the evidence and the handoff cross-check refer to.
    pub name: String,
    pub argv: Vec<String>,
    pub timeout_secs: u64,
}

/// The bounded tail of combined stdout+stderr, so a huge build log cannot
/// bloat the durable record. The head is marked as trimmed.
const MAX_TAIL_CHARS: usize = 4_000;

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
    let command_display = gate.argv.join(" ");
    let started = Instant::now();
    let mut cmd = tokio::process::Command::new(&gate.argv[0]);
    cmd.args(&gate.argv[1..]).current_dir(workdir);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // no console window flash

    let outcome = tokio::time::timeout(Duration::from_secs(gate.timeout_secs.max(1)), async {
        let child = cmd.spawn().map_err(|e| e.to_string())?;
        let output = child.wait_with_output().await.map_err(|e| e.to_string())?;
        Ok::<_, String>(output)
    })
    .await;

    let duration_ms = started.elapsed().as_millis() as u64;
    match outcome {
        Err(_elapsed) => QualityEvidence {
            gate: gate.name.clone(),
            command: command_display,
            passed: false,
            exit_code: None,
            timed_out: true,
            duration_ms,
            log_tail: format!("gate exceeded {}s and was terminated", gate.timeout_secs),
        },
        Ok(Err(spawn_error)) => QualityEvidence {
            gate: gate.name.clone(),
            command: command_display,
            passed: false,
            exit_code: None,
            timed_out: false,
            duration_ms,
            log_tail: format!("could not start the gate command: {spawn_error}"),
        },
        Ok(Ok(output)) => {
            let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&output.stderr));
            let log_tail = if combined.len() > MAX_TAIL_CHARS {
                let mut tail = combined[combined.len() - MAX_TAIL_CHARS..].to_string();
                while !tail.is_char_boundary(0) {
                    tail.remove(0);
                }
                format!("[…trimmed…]{tail}")
            } else {
                combined
            };
            QualityEvidence {
                gate: gate.name.clone(),
                command: command_display,
                passed: output.status.success(),
                exit_code: output.status.code(),
                timed_out: false,
                duration_ms,
                log_tail,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let evidence = run_gate(Path::new("."), &gate).await;
        assert!(!evidence.passed);
        assert!(evidence.timed_out);
        assert_eq!(evidence.exit_code, None);
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
}

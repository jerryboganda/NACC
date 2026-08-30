//! Native Windows / WSL2 / Docker runtime detection and execution
//! abstraction (master plan S1, S11's "runtime" role-row field, S17.1's
//! Setup Wizard prerequisite detection; Phase 3 scope).
//!
//! Detection functions here treat "the tool isn't installed" as a normal,
//! valid, typed result -- not an error. A user's machine genuinely may
//! not have WSL2 or Docker; that is exactly the fact the Setup Wizard
//! needs reported, not an exception to catch. A *different*, unexpected
//! I/O failure still propagates as a real [`RuntimeError`].
//!
//! **`wsl.exe` output encoding, verified not assumed**: `wsl.exe` writes
//! UTF-16LE to a redirected/piped stdout by default -- a real,
//! long-documented Windows/WSL interop quirk (see e.g.
//! `microsoft/WSL#7767`, `microsoft/WSL#5063`) that has bitten other
//! tools (VS Code's own WSL integration has open issues about exactly
//! this). Setting the `WSL_UTF8=1` environment variable on the child
//! process switches it to plain UTF-8, which this crate always does --
//! sidestepping the encoding ambiguity entirely rather than trying to
//! detect or handle both encodings.
//!
//! **Honesty about what could not be live-verified**: this machine has no
//! WSL2 or Docker install to test against (see the Phase 0 foundation
//! audit), and GitHub Actions' `windows-latest` runner's WSL2/Docker/VS
//! Code availability is not something this crate controls or can assume.
//! Parsing logic is unit-tested against realistic sample output matching
//! Microsoft's documented format, not a live install -- stated plainly
//! rather than implied to be more verified than it is.

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("failed to spawn {program}: {source}")]
    Spawn {
        program: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("{program} {args:?} failed (exit {exit_code:?}): {stderr}")]
    CommandFailed {
        program: &'static str,
        args: Vec<String>,
        exit_code: Option<i32>,
        stderr: String,
    },
    #[error("VS Code is not installed (tried `code` and `code-insiders`)")]
    VsCodeNotFound,
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

/// Where a command should actually execute. `NativeWindows` is the
/// default and only target real today; `Wsl2` is real detection plus a
/// real command-wrapping transform. Docker execution wrapping and any
/// "configured remote runtime" (master plan S11's row-field list also
/// names this) are deliberately not modeled yet -- Docker's own volume
/// mount / working-directory mapping needs real design work this phase
/// does not scope, and no remote-runtime concept exists anywhere else in
/// the codebase yet to model faithfully.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeTarget {
    NativeWindows,
    Wsl2 { distro: String },
}

/// Result of probing for WSL2. `Available` with an empty `distro_names`
/// is a real, distinct state from `NotAvailable`: the WSL2 feature/CLI is
/// present, but no Linux distribution is installed under it yet (a very
/// common fresh-Windows-install state) -- the Setup Wizard needs to tell
/// these two apart to give the right guidance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Wsl2Status {
    NotAvailable,
    Available { distro_names: Vec<String> },
}

/// Result of probing for Docker. `daemon_reachable: false` with the CLI
/// present and a real version is the single most common real-world state
/// (Docker Desktop installed but not currently running) -- distinct from
/// `NotAvailable` (the `docker` CLI itself is not installed at all).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DockerStatus {
    NotAvailable,
    Available {
        version: String,
        daemon_reachable: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VsCodeInstallation {
    /// `"code"` or `"code-insiders"` -- whichever was actually found.
    pub executable: &'static str,
    pub version: String,
}

fn new_command(program: &'static str) -> tokio::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    tokio::process::Command::from(cmd)
}

/// Search `PATH` (as given, so this is unit-testable without touching
/// the real process environment) for a file literally named `filename`.
fn find_on_path_in(filename: &str, path_var: &std::ffi::OsStr) -> Option<std::path::PathBuf> {
    std::env::split_paths(path_var)
        .map(|dir| dir.join(filename))
        .find(|candidate| candidate.is_file())
}

fn find_on_path(filename: &str) -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    find_on_path_in(filename, &path_var)
}

/// Build a command that runs a Windows `.cmd`/`.bat` launcher script via
/// an explicit `cmd.exe /C`, rather than relying on
/// `std::process::Command`'s historical `CreateProcessW`-level fallback
/// that auto-converts a bare `.cmd` path into a `cmd.exe` invocation
/// internally -- real, but documented by Rust itself as legacy behavior
/// that "should not be relied upon" and may be removed. Returns `None`
/// if `script_name` is not found on `PATH` at all, resolved by an
/// explicit filesystem search rather than by spawning `cmd.exe` and
/// inspecting its (locale-dependent) "not recognized" error text.
fn new_cmd_script_command(script_name: &str) -> Option<tokio::process::Command> {
    let script_path = find_on_path(script_name)?;
    let mut cmd = std::process::Command::new("cmd.exe");
    cmd.arg("/C").arg(&script_path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    Some(tokio::process::Command::from(cmd))
}

/// Runs `program` with `args`, returning `Ok(None)` if `program` is not
/// found on PATH at all (a normal "not installed" outcome, not an
/// error), `Ok(Some(stdout))` on success, and `Err` for a real,
/// unexpected failure (including the program running but exiting
/// nonzero, which is surfaced as [`RuntimeError::CommandFailed`] rather
/// than folded into "not installed" -- callers decide whether a nonzero
/// exit means "not installed" for their specific tool, since it means
/// different things for `wsl -l -q` versus `docker --version`).
async fn run_and_classify(
    program_label: &'static str,
    mut cmd: tokio::process::Command,
    args_for_error: &[&str],
) -> Result<Option<String>> {
    let output = match cmd.output().await {
        Ok(output) => output,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(RuntimeError::Spawn {
                program: program_label,
                source: e,
            })
        }
    };
    if !output.status.success() {
        return Err(RuntimeError::CommandFailed {
            program: program_label,
            args: args_for_error.iter().map(|s| s.to_string()).collect(),
            exit_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
}

async fn try_run(program: &'static str, args: &[&str], utf8_env: bool) -> Result<Option<String>> {
    let mut cmd = new_command(program);
    cmd.args(args);
    if utf8_env {
        cmd.env("WSL_UTF8", "1");
    }
    run_and_classify(program, cmd, args).await
}

/// Like [`try_run`], but for a Windows `.cmd`/`.bat` launcher script
/// (see [`new_cmd_script_command`]) rather than a real executable.
async fn try_run_cmd_script(script_name: &'static str, args: &[&str]) -> Result<Option<String>> {
    let Some(mut cmd) = new_cmd_script_command(script_name) else {
        return Ok(None);
    };
    cmd.args(args);
    run_and_classify(script_name, cmd, args).await
}

/// Probe for WSL2 via `wsl.exe -l -q` (plain distro names, one per line
/// -- deliberately not `-l -v`'s columnar state/version/default-marker
/// format, which this crate cannot validate against a real install and
/// whose exact spacing/locale behavior is less consistently documented).
pub async fn detect_wsl2() -> Result<Wsl2Status> {
    match try_run("wsl.exe", &["-l", "-q"], true).await {
        Ok(Some(output)) => Ok(Wsl2Status::Available {
            distro_names: parse_distro_names(&output),
        }),
        Ok(None) => Ok(Wsl2Status::NotAvailable),
        // `wsl.exe` present but with zero distros installed exits nonzero
        // with a message directing the user to `wsl --install` -- a
        // real, normal "available, nothing installed yet" state, not an
        // error.
        Err(RuntimeError::CommandFailed { .. }) => Ok(Wsl2Status::Available {
            distro_names: Vec::new(),
        }),
        Err(e) => Err(e),
    }
}

fn parse_distro_names(output: &str) -> Vec<String> {
    output
        .lines()
        // A leading UTF-8 BOM has been observed from `wsl.exe` even with
        // WSL_UTF8=1 set on some builds; trim it defensively per line
        // rather than assume it only ever appears once at the very start.
        .map(|line| line.trim_start_matches('\u{feff}').trim())
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Probe for Docker: CLI presence/version via `docker --version` (a
/// single stable line, unlike `docker info`'s large and potentially
/// slow, daemon-dependent output), then daemon reachability separately
/// via `docker info` so a not-currently-running Docker Desktop reports
/// as `daemon_reachable: false`, not `NotAvailable`.
pub async fn detect_docker() -> Result<DockerStatus> {
    let version = match try_run("docker.exe", &["--version"], false).await {
        Ok(Some(output)) => output.trim().to_string(),
        Ok(None) => return Ok(DockerStatus::NotAvailable),
        Err(e) => return Err(e),
    };
    let daemon_reachable = matches!(
        try_run(
            "docker.exe",
            &["info", "--format", "{{.ServerVersion}}"],
            false
        )
        .await,
        Ok(Some(_))
    );
    Ok(DockerStatus::Available {
        version,
        daemon_reachable,
    })
}

/// Probe for VS Code, trying the stable build first, then Insiders.
/// VS Code's own installer puts a `.cmd` launcher shim (confirmed
/// against VS Code's own docs: `...\Microsoft VS Code\bin\code.cmd`),
/// not a bare `.exe`, on `PATH` on Windows -- see
/// [`new_cmd_script_command`] for why this is invoked via an explicit
/// `cmd.exe /C` rather than directly.
pub async fn detect_vscode() -> Result<Option<VsCodeInstallation>> {
    for (executable, script) in [("code", "code.cmd"), ("code-insiders", "code-insiders.cmd")] {
        if let Some(output) = try_run_cmd_script(script, &["--version"]).await? {
            let version = output.lines().next().unwrap_or_default().trim().to_string();
            return Ok(Some(VsCodeInstallation {
                executable,
                version,
            }));
        }
    }
    Ok(None)
}

/// Open `path` in VS Code (master plan S16, S17.3: "open primary
/// checkout or any worktree in VS Code"). Fire-and-forget: `code` hands
/// off to an existing VS Code instance (or spawns a new window) and
/// returns immediately, so this does not wait for the editor to close.
pub async fn launch_vscode(path: &Path) -> Result<()> {
    for script in ["code.cmd", "code-insiders.cmd"] {
        let Some(mut cmd) = new_cmd_script_command(script) else {
            continue;
        };
        cmd.arg(path);
        cmd.spawn().map_err(|e| RuntimeError::Spawn {
            program: "code",
            source: e,
        })?;
        return Ok(());
    }
    Err(RuntimeError::VsCodeNotFound)
}

/// Transform a `program`/`args` pair into whatever actually needs to be
/// spawned to run it under `target`. Pure and synchronous -- no
/// subprocess involved, so this is exactly as testable as any other
/// string transformation.
pub fn wrap_for_target(
    target: &RuntimeTarget,
    program: &str,
    args: &[String],
) -> (String, Vec<String>) {
    match target {
        RuntimeTarget::NativeWindows => (program.to_string(), args.to_vec()),
        RuntimeTarget::Wsl2 { distro } => {
            let mut wrapped = vec![
                "-d".to_string(),
                distro.clone(),
                "--".to_string(),
                program.to_string(),
            ];
            wrapped.extend(args.iter().cloned());
            ("wsl.exe".to_string(), wrapped)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_distro_names_splits_plain_lines() {
        let names = parse_distro_names("Ubuntu-22.04\nDebian\n");
        assert_eq!(
            names,
            vec!["Ubuntu-22.04".to_string(), "Debian".to_string()]
        );
    }

    #[test]
    fn parse_distro_names_ignores_blank_lines_and_a_leading_bom() {
        let names = parse_distro_names("\u{feff}Ubuntu-22.04\n\n");
        assert_eq!(names, vec!["Ubuntu-22.04".to_string()]);
    }

    #[test]
    fn parse_distro_names_of_empty_output_is_an_empty_list_not_an_error() {
        assert!(parse_distro_names("").is_empty());
    }

    #[test]
    fn find_on_path_in_finds_a_real_file_in_a_searched_directory() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("nacc-runtime-test-find-on-path-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("nacc-fake-code.cmd");
        std::fs::write(&script, "@echo off\n").unwrap();

        let path_var = std::env::join_paths([dir.as_path()]).unwrap();
        let found = find_on_path_in("nacc-fake-code.cmd", &path_var);

        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(found, Some(script));
    }

    #[test]
    fn find_on_path_in_does_not_find_a_nonexistent_file_in_a_real_directory() {
        let dir = std::env::temp_dir();
        let path_var = std::env::join_paths([dir.as_path()]).unwrap();
        assert!(find_on_path_in("nacc-definitely-does-not-exist.cmd", &path_var).is_none());
    }

    #[test]
    fn wrap_for_target_native_windows_is_the_identity_transform() {
        let (program, args) = wrap_for_target(
            &RuntimeTarget::NativeWindows,
            "claude",
            &["--effort".to_string(), "high".to_string()],
        );
        assert_eq!(program, "claude");
        assert_eq!(args, vec!["--effort".to_string(), "high".to_string()]);
    }

    #[test]
    fn wrap_for_target_wsl2_wraps_with_distro_and_separator() {
        let (program, args) = wrap_for_target(
            &RuntimeTarget::Wsl2 {
                distro: "Ubuntu-22.04".to_string(),
            },
            "claude",
            &["--effort".to_string(), "high".to_string()],
        );
        assert_eq!(program, "wsl.exe");
        assert_eq!(
            args,
            vec![
                "-d".to_string(),
                "Ubuntu-22.04".to_string(),
                "--".to_string(),
                "claude".to_string(),
                "--effort".to_string(),
                "high".to_string(),
            ]
        );
    }

    // The following exercise the real, live detection functions. They
    // deliberately assert only that detection completes and returns a
    // well-typed result, never a specific installed/not-installed
    // outcome -- this crate does not control, and must not assume,
    // whether the machine running these tests (this workspace's own CI
    // included) has WSL2, Docker, or VS Code installed.

    #[tokio::test]
    async fn detect_wsl2_completes_without_erroring() {
        let status = detect_wsl2().await;
        assert!(
            status.is_ok(),
            "detect_wsl2 must classify, not error, on a normal machine: {status:?}"
        );
    }

    #[tokio::test]
    async fn detect_docker_completes_without_erroring() {
        let status = detect_docker().await;
        assert!(
            status.is_ok(),
            "detect_docker must classify, not error, on a normal machine: {status:?}"
        );
    }

    #[tokio::test]
    async fn detect_vscode_completes_without_erroring() {
        let installation = detect_vscode().await;
        assert!(
            installation.is_ok(),
            "detect_vscode must classify, not error, on a normal machine: {installation:?}"
        );
    }
}

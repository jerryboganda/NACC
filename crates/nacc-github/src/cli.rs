//! Thin, safe wrappers around the installed `gh` CLI (master plan S19's
//! GitHub integration, reached through the user's own authenticated `gh`
//! rather than NACC-owned tokens). Only ever invoked with fixed argument
//! lists -- never with interpolated shell.

use std::process::Stdio;

use crate::{GithubError, Result};

async fn run_gh(args: &[&str]) -> Result<String> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args(args);
    cmd.stdin(Stdio::null());
    // Windows-only: no console window flashing for a background probe.
    // tokio::process::Command carries creation_flags natively on Windows.
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let output = cmd.output().await.map_err(|e| {
        GithubError::Other(format!(
            "could not run `gh` (is GitHub CLI installed and on PATH?): {e}"
        ))
    })?;
    if !output.status.success() {
        return Err(GithubError::Other(format!(
            "`gh {}` failed ({}): {}",
            args.join(" "),
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The installed `gh`'s own version string, for the Setup Wizard's
/// prerequisite detection (master plan S17.1: "Show exact executable paths
/// and versions"). An absent `gh` is a normal outcome, not an error: callers
/// get `Ok(None)`-style information through [`crate::gh_installed`].
pub async fn gh_version() -> Result<String> {
    Ok(run_gh(&["--version"])
        .await?
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string())
}

/// Whether `gh` is runnable at all -- the Setup Wizard's honest "detected /
/// not detected" bit, distinct from "authenticated" (which only `gh auth
/// status` can answer and which NACC reads without exposing its output
/// contents beyond the typed summary).
pub async fn gh_installed() -> bool {
    matches!(gh_version().await, Ok(version) if !version.is_empty())
}

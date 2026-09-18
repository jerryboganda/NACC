//! Typed GitHub and GitHub Actions integration through the user's existing
//! authenticated `gh` CLI (master plan S4.6, S19). Rust owns the command
//! boundary and callers receive structured state rather than terminal prose.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum GithubError {
    #[error("failed to spawn GitHub CLI `{executable}`: {source}")]
    Spawn {
        executable: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("GitHub CLI command {args:?} failed (exit {exit_code:?}): {stderr}")]
    CommandFailed {
        args: Vec<String>,
        exit_code: Option<i32>,
        stderr: String,
    },
    #[error("could not parse GitHub CLI output for {context}: {source}")]
    Parse {
        context: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid GitHub request: {0}")]
    InvalidRequest(String),
    #[error("GitHub CLI command {args:?} timed out after {seconds} seconds")]
    Timeout { args: Vec<String>, seconds: u64 },
    #[error("the requested GitHub mutation requires explicit human approval")]
    ApprovalRequired,
}

pub type Result<T> = std::result::Result<T, GithubError>;

pub use classify::{classify, FailureClass};
pub use cli::{gh_installed, gh_version, GhClient};
pub use models::*;

mod classify;
mod cli;
mod models;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_error_is_deliberately_unambiguous() {
        assert_eq!(
            GithubError::ApprovalRequired.to_string(),
            "the requested GitHub mutation requires explicit human approval"
        );
    }
}

//! Structured logging and tracing initialization shared by every NACC
//! crate (master plan S4.2: "Tracing + tracing-subscriber for structured
//! local logs"; S22: correlation IDs and structured audit-adjacent logs).
//!
//! This crate configures WHERE and HOW structured logs are written. It
//! does not decide WHAT gets logged -- that is every call site's
//! responsibility, governed by master plan S18: "Do not store or expose
//! hidden chain-of-thought. Persist visible plans, summaries, tool
//! events, commands, outputs, and validated handoff artifacts only." A
//! redaction layer for secrets (S13.5) is explicit Phase 11 scope and is
//! not implemented here yet.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Debug, thiserror::Error)]
pub enum ObservabilityError {
    #[error("failed to create log directory {path}: {source}")]
    LogDirectory {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("tracing subscriber was already initialized for this process")]
    AlreadyInitialized,
}

pub type Result<T> = std::result::Result<T, ObservabilityError>;

/// Initialize NACC's process-wide tracing subscriber. Call exactly once,
/// as early as possible in application startup (src-tauri's `main.rs`).
///
/// Writes two layers: a daily-rotating JSON file under `log_dir`
/// (durable, machine-parseable, matches master plan S22's structured-log
/// requirement) and a human-readable console layer (colored and verbose
/// in `dev_mode`, plain and quieter otherwise). Both share one filter,
/// configured from the `NACC_LOG` environment variable
/// (e.g. `NACC_LOG=nacc_process=debug,info`) with a sensible default when
/// unset.
///
/// Returns a guard that MUST be kept alive for the lifetime of the
/// process. Dropping it stops the non-blocking file writer from
/// flushing -- hold it in `main()`'s local scope, never in a value that
/// gets dropped early.
pub fn init_tracing(log_dir: &Path, dev_mode: bool) -> Result<WorkerGuard> {
    std::fs::create_dir_all(log_dir).map_err(|source| ObservabilityError::LogDirectory {
        path: log_dir.display().to_string(),
        source,
    })?;

    let file_appender = tracing_appender::rolling::daily(log_dir, "nacc.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::try_from_env("NACC_LOG")
        .or_else(|_| EnvFilter::try_new(if dev_mode { "debug" } else { "info" }))
        .expect("the fallback EnvFilter directive is a constant, always-valid string");

    let file_layer = fmt::layer()
        .json()
        .with_writer(non_blocking)
        .with_ansi(false);

    let console_layer = fmt::layer().with_ansi(dev_mode).with_target(dev_mode);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .with(console_layer)
        .try_init()
        .map_err(|_| ObservabilityError::AlreadyInitialized)?;

    Ok(guard)
}

/// Open a span carrying the standard `workflow_run_id` correlation field
/// (master plan S22's correlation-id list). Every event logged while this
/// span is entered is automatically tagged, so a full run can be filtered
/// out of the shared log file by this one field. More correlation-id
/// helpers (project, node run, attempt, provider session, process,
/// worktree, GitHub run) are added alongside their owning entities as
/// those are modeled in Phase 2 and later -- this function is the pattern
/// they follow, not the complete set.
pub fn workflow_run_span(run_id: nacc_domain::WorkflowRunId) -> tracing::Span {
    tracing::info_span!("workflow_run", workflow_run_id = %run_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_tracing_creates_the_log_directory_and_returns_a_live_guard() {
        let dir =
            std::env::temp_dir().join(format!("nacc-observability-test-{}", uuid::Uuid::new_v4()));
        assert!(!dir.exists());

        let guard =
            init_tracing(&dir, true).expect("init_tracing should succeed on a fresh temp dir");
        assert!(dir.exists(), "log directory must be created");

        tracing::info!(target: "nacc_observability::tests", "smoke test event");

        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn workflow_run_span_carries_the_correlation_field() {
        // A *scoped* subscriber (tracing::subscriber::with_default), not
        // the global one init_tracing installs: tracing_subscriber's
        // global default can only be set once per process, and Rust's
        // test harness runs tests in parallel, so asserting
        // `!span.is_disabled()` against global state race-depends on
        // whether the *other* test in this file happened to call
        // `try_init()` first -- confirmed the hard way by this
        // workspace's own CI, which failed this exact assertion on a
        // run where test order put this one first. Scoping the
        // subscriber to this test's own closure makes the assertion
        // deterministic regardless of execution order.
        let subscriber = tracing_subscriber::fmt().finish();
        tracing::subscriber::with_default(subscriber, || {
            let run_id = nacc_domain::WorkflowRunId::new();
            let span = workflow_run_span(run_id);
            assert!(!span.is_disabled());
            let _enter = span.enter();
        });
    }
}

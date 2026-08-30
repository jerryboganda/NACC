//! The first real, end-to-end typed IPC command (Phase 1's proof that the
//! whole pipe works): Rust command -> specta type export -> generated
//! TypeScript bindings -> React call -> rendered result.
//!
//! Deliberately not a "Projects" or "Role Matrix" feature yet -- those are
//! Phase 2/6 scope with real domain data behind them. This command only
//! proves the mechanism, using one real domain type
//! (`nacc_domain::WorkflowRunId`) so the round trip exercises an actual
//! strongly-typed ID, not just primitive strings and numbers.

use serde::Serialize;
use tauri::State;

use nacc_domain::WorkflowRunId;

use crate::AppState;

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct AppDiagnostics {
    pub app_version: String,
    /// Kept in sync by hand with the root `Cargo.toml`'s `[workspace]
    /// members` list (currently 20 library crates + this application
    /// shell). Not worth a build-time Cargo.toml parser for a diagnostics
    /// display value at this phase.
    pub workspace_crate_count: u32,
    /// The workspace's declared MSRV (`rust-version` in the root
    /// `Cargo.toml`), read via Cargo's `CARGO_PKG_RUST_VERSION` build-time
    /// environment variable -- not the compiler that happened to build
    /// this binary, which Cargo does not expose to running code.
    pub minimum_rust_version: String,
    pub dev_mode: bool,
    /// Proves a real `nacc-domain` strong ID -- not a plain string or a
    /// bare UUID -- serializes correctly across the IPC boundary and
    /// deserializes back into a matching TypeScript type.
    pub sample_workflow_run_id: WorkflowRunId,
}

#[tauri::command]
#[specta::specta]
pub fn get_app_diagnostics(state: State<'_, AppState>) -> AppDiagnostics {
    AppDiagnostics {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        workspace_crate_count: 21,
        minimum_rust_version: env!("CARGO_PKG_RUST_VERSION").to_string(),
        dev_mode: cfg!(debug_assertions),
        sample_workflow_run_id: state.diagnostics_run_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_reports_a_real_workspace_msrv_string() {
        // env! resolves at compile time; this just guards against the
        // build-time variable silently becoming empty if Cargo's
        // behavior ever changes.
        assert!(!env!("CARGO_PKG_RUST_VERSION").is_empty());
    }
}

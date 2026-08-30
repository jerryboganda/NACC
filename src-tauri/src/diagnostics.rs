//! The first real, end-to-end typed IPC command (Phase 1's proof that the
//! whole pipe works): Rust command -> specta type export -> generated
//! TypeScript bindings -> React call -> rendered result. Phase 2 extends
//! it into a real storage round trip too: every call writes one audit
//! record through `nacc-storage` and reads back how many this session has
//! recorded, so the count in the returned struct is live data, not a
//! placeholder -- watchable by simply reopening the diagnostics view.
//!
//! Deliberately not a "Projects" or "Role Matrix" feature yet -- those are
//! Phase 6 scope with real domain data behind them. This command only
//! proves the mechanism, using real domain types
//! (`nacc_domain::WorkflowRunId`, `nacc_events::AuditRecord`) so the round
//! trip exercises actual strongly-typed IDs and real persistence, not just
//! primitive strings and numbers.

use serde::Serialize;
use tauri::State;

use nacc_domain::WorkflowRunId;
use nacc_events::AuditRecord;

use crate::{now_millis, AppState};

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct AppDiagnostics {
    pub app_version: String,
    /// Kept in sync by hand with the root `Cargo.toml`'s `[workspace]
    /// members` list (currently 21 library crates + this application
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
    /// The applied SQLite schema version (master plan S4.4), formatted for
    /// display. Proves `nacc-storage`'s migrations genuinely ran against
    /// the real on-disk database this process opened, not an in-memory
    /// test double.
    pub storage_schema_version: String,
    /// How many audit records this command has ever written for
    /// `sample_workflow_run_id`, read back fresh on every call. A real,
    /// live query result: it increments by one each time this command
    /// runs, which is directly observable by calling it twice in the same
    /// running app -- not a static or precomputed value.
    pub diagnostics_requests_recorded: u32,
}

#[tauri::command]
#[specta::specta]
pub async fn get_app_diagnostics(state: State<'_, AppState>) -> Result<AppDiagnostics, String> {
    let storage = state.storage.clone();

    let record = AuditRecord::new(
        "diagnostics_command".to_string(),
        "get_app_diagnostics".to_string(),
        None,
        Some(state.diagnostics_run_id),
        None,
        None,
        now_millis(),
    );
    storage
        .append_audit_record(&record)
        .await
        .map_err(|e| e.to_string())?;

    let recorded = storage
        .list_audit_records_for_workflow_run(state.diagnostics_run_id)
        .await
        .map_err(|e| e.to_string())?;

    let storage_schema_version = match storage.schema_version().map_err(|e| e.to_string())? {
        rusqlite_migration::SchemaVersion::NoneSet => "none".to_string(),
        rusqlite_migration::SchemaVersion::Inside(n) => n.to_string(),
        // A real, meaningful case, not a stub: the database's applied
        // user_version is higher than any migration this build knows
        // about -- e.g. a newer NACC version wrote it, then an older
        // binary opened it (a real CI finding: `rusqlite_migration`
        // 2.6.0's `SchemaVersion` has three variants, not the two an
        // earlier docs.rs summary reported). Surfaced distinctly rather
        // than folded into "Inside" so this is diagnosable from the
        // diagnostics screen itself, not just from a crash.
        rusqlite_migration::SchemaVersion::Outside(n) => format!("outside ({n})"),
    };

    Ok(AppDiagnostics {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        workspace_crate_count: 21,
        minimum_rust_version: env!("CARGO_PKG_RUST_VERSION").to_string(),
        dev_mode: cfg!(debug_assertions),
        sample_workflow_run_id: state.diagnostics_run_id,
        storage_schema_version,
        diagnostics_requests_recorded: recorded.len() as u32,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn diagnostics_reports_a_real_workspace_msrv_string() {
        // env! resolves at compile time; this just guards against the
        // build-time variable silently becoming empty if Cargo's
        // behavior ever changes.
        assert!(!env!("CARGO_PKG_RUST_VERSION").is_empty());
    }
}

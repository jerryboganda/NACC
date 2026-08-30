//! NACC application shell: the Tauri 2 composition root. Wires typed IPC
//! (tauri-specta), tracing initialization (`nacc-observability`), and
//! (eventually, in later phases) every domain crate together behind the
//! webview boundary.
//!
//! Master plan S2.2: "The React layer presents state and sends typed
//! commands. It must not become a hidden Node.js backend." Everything
//! privileged lives in this crate and the ones it depends on.

mod diagnostics;

use tauri::Manager;
use tauri_specta::{collect_commands, Builder};

/// Application-managed state. Phase 1 holds only what the diagnostics
/// command and tracing lifecycle need; real domain state (the provider
/// registry, workflow engine handle, SQLite pool, ...) is added crate by
/// crate as each is wired in during later phases.
pub struct AppState {
    /// A placeholder value for the Phase 1 diagnostics command only --
    /// proves a real `nacc-domain` strong ID flows through typed IPC. Not
    /// a real workflow run; nothing creates workflow runs yet.
    pub diagnostics_run_id: nacc_domain::WorkflowRunId,
    /// Must outlive the app for the non-blocking file log writer to keep
    /// flushing -- see `nacc_observability::init_tracing`'s doc comment.
    _tracing_guard: tracing_appender::non_blocking::WorkerGuard,
}

/// Builds the tauri-specta `Builder` used both to generate TypeScript
/// bindings and to construct the app's `invoke_handler`. Factored out of
/// `run()` so `main()`'s `--export-bindings` flag can call the exact same
/// construction the running app uses to (re)generate the gitignored
/// `src/bindings.ts` the frontend imports -- see `main.rs` for why that
/// flag exists instead of a `tests/` integration test.
pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![diagnostics::get_app_diagnostics])
}

/// Absolute path to `src/bindings.ts`, resolved from `CARGO_MANIFEST_DIR`
/// (compile-time, always this crate's own directory) rather than a
/// runtime-relative literal -- `cargo run` does not change the process's
/// working directory to the target package's directory the way `cargo
/// test` does, so a bare `"../src/bindings.ts"` resolves differently (and
/// wrongly) depending on where the binary was launched from. Confirmed
/// the hard way in CI: see `main.rs`'s `--export-bindings` handler, the
/// other caller of this function.
pub fn bindings_output_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/bindings.ts")
}

pub fn run() {
    let builder = specta_builder();

    #[cfg(debug_assertions)]
    builder
        .export(
            specta_typescript::Typescript::default(),
            bindings_output_path(),
        )
        .expect("failed to export TypeScript bindings");

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);

            let log_dir = app
                .path()
                .app_log_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("./logs"));
            let guard = nacc_observability::init_tracing(&log_dir, cfg!(debug_assertions))
                .expect("failed to initialize tracing");

            let diagnostics_run_id = nacc_domain::WorkflowRunId::new();
            // Real usage of the correlation-span helper (not just its own
            // test) -- every startup log line below carries
            // workflow_run_id, demonstrating the pattern later phases
            // repeat for project/node-run/attempt/provider-session/etc.
            // correlation IDs (master plan S22).
            let _startup_span = nacc_observability::workflow_run_span(diagnostics_run_id).entered();

            tracing::info!(
                app_version = env!("CARGO_PKG_VERSION"),
                dev_mode = cfg!(debug_assertions),
                "NACC starting up"
            );

            app.manage(AppState {
                diagnostics_run_id,
                _tracing_guard: guard,
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the NACC application");
}

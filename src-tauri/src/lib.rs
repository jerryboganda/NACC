//! NACC application shell: the Tauri 2 composition root. Wires typed IPC
//! (tauri-specta), tracing initialization (`nacc-observability`), and
//! (eventually, in later phases) every domain crate together behind the
//! webview boundary.
//!
//! Master plan S2.2: "The React layer presents state and sends typed
//! commands. It must not become a hidden Node.js backend." Everything
//! privileged lives in this crate and the ones it depends on.

mod diagnostics;
mod executor;
mod providers;
mod role_profiles;
mod routing;
mod workflows;

use tauri::Manager;
use tauri_specta::{collect_commands, Builder};

/// Application-managed state. Real domain state beyond storage (the
/// provider registry, workflow engine handle, ...) is added crate by crate
/// as each is wired in during later phases.
pub struct AppState {
    /// A placeholder value for the diagnostics command only -- proves a
    /// real `nacc-domain` strong ID flows through typed IPC, and doubles
    /// as the correlation id the startup audit record below is filed
    /// under. Not a real workflow run; nothing creates workflow runs yet.
    pub diagnostics_run_id: nacc_domain::WorkflowRunId,
    /// NACC's one local SQLite database (master plan S4.4), opened once
    /// here and cloned (cheaply -- an `Arc` clone) into every command that
    /// needs storage access. See `nacc_storage`'s crate doc for why one
    /// shared connection behind a mutex, not a pool, is the right shape
    /// for a single-process desktop app.
    pub storage: nacc_storage::Database,
    /// The real provider adapters this build can probe (and launch).
    /// Populated once in `.setup()` via `providers::build_registry` -- see
    /// that module's doc comment for why only Claude and Codex are
    /// registered today.
    pub providers: std::sync::Arc<nacc_provider_core::ProviderRegistry>,
    /// The Role Matrix as the engine's routing table, refreshed whenever role
    /// profiles change. Shared with the executor so a role cannot route to one
    /// model and launch with another.
    pub routing: std::sync::Arc<routing::RoleMatrixRouting>,
    /// The durable workflow engine (Phase 7), wired into the application so
    /// real runs can be started and resumed.
    pub engine: std::sync::Arc<nacc_orchestrator::WorkflowEngine>,
    /// Worktree lease management for this process: allocation, inspection,
    /// release, and quarantine of per-run worktrees (master plan S16).
    pub worktrees: std::sync::Arc<nacc_worktree::WorktreeManager>,
    /// Run id -> the lease allocated for it in this session. The durable
    /// lease rows in storage are the source of truth (startup reconciliation
    /// reads those); this map exists so a run that finishes or is cancelled
    /// while NACC is open releases its worktree promptly.
    pub run_leases: std::sync::Arc<
        std::sync::Mutex<
            std::collections::HashMap<nacc_domain::WorkflowRunId, nacc_domain::WorktreeLease>,
        >,
    >,
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
    Builder::<tauri::Wry>::new().commands(collect_commands![
        diagnostics::get_app_diagnostics,
        role_profiles::list_role_profiles,
        role_profiles::create_role_profile,
        role_profiles::update_role_profile,
        role_profiles::set_role_profile_enabled,
        role_profiles::delete_role_profile,
        providers::detect_provider,
        providers::list_provider_installations,
        providers::check_provider_auth,
        workflows::list_workflow_templates,
        workflows::start_workflow_run,
        workflows::get_workflow_run,
        workflows::list_workflow_runs,
        workflows::pause_workflow_run,
        workflows::cancel_workflow_run,
        workflows::resume_workflow_run,
        workflows::decide_workflow_approval,
        workflows::list_workflow_events,
        workflows::save_workflow_template,
        workflows::delete_workflow_template,
    ])
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

/// Milliseconds since the Unix epoch, used by `diagnostics.rs`'s real
/// storage round trip. Each crate that needs one defines its own small
/// copy rather than depending on a shared public utility crate for a
/// single-line computation -- see `nacc_provider_core::capability`'s
/// test-only version and `nacc_storage`'s `pub(crate)` version for the
/// same pattern, and this workspace's own CI history (`error: function
/// 'now_millis' is never used`) for why a *public*, cross-crate version
/// would be actively worse than this duplication.
pub(crate) fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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

            // `Database::open` is a plain synchronous function (see its
            // own doc comment: opening and migrating are startup-only
            // work, not on any hot path), so it's safe to call directly
            // here -- unlike an async storage call, which must NOT be
            // driven with `tauri::async_runtime::block_on` from inside
            // `.setup()`: `.setup()` already runs inside Tauri's own
            // Tokio runtime, and nesting a second blocking runtime inside
            // a running one deadlocks/panics (confirmed against Tauri's
            // own issue tracker, not assumed -- see `diagnostics.rs` for
            // where this workspace's actual async storage round trip
            // lives instead, entirely inside one already-async command).
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("./data"));
            // `nacc_storage::Database::open` itself logs the schema
            // version and whether it changed (a library-level DEBUG
            // event, correct for any future caller, not just this one);
            // this line is the higher-level "the app is up and storage is
            // ready" confirmation that belongs at the application's own
            // INFO level.
            let storage = nacc_storage::Database::open(&data_dir.join("nacc.sqlite"))
                .expect("failed to open NACC database");
            tracing::info!(db_path = %data_dir.join("nacc.sqlite").display(), "NACC storage ready");

            // The engine, its executor, and the routing table are one object
            // graph: the executor and the engine share the routing snapshot, so
            // "which provider does this role use" has exactly one answer at
            // launch time as well as at scheduling time.
            let providers = std::sync::Arc::new(providers::build_registry());
            let routing = std::sync::Arc::new(routing::RoleMatrixRouting::default());
            let executor = std::sync::Arc::new(executor::ProviderNodeExecutor::new(
                providers.clone(),
                routing.clone(),
                storage.clone(),
            ));
            let engine = std::sync::Arc::new(nacc_orchestrator::WorkflowEngine::with_defaults(
                std::sync::Arc::new(storage.clone()),
                executor,
                routing.clone(),
            ));
            // The worktree manager shares storage (durable lease rows) and is
            // tagged with this process id, which is how a later start can
            // tell "still running" from "orphaned by a crash" (master plan
            // S16's reconciliation rule).
            let worktrees = std::sync::Arc::new(
                nacc_worktree::WorktreeManager::new(storage.clone())
                    .with_owner_process_id(std::process::id()),
            );

            // Load the persisted Role Matrix into the routing snapshot. Done as
            // a task rather than inline because storage is async and `.setup()`
            // must not block on I/O (see the `Database::open` note above);
            // until it completes, the snapshot is empty, which means "nothing is
            // routable yet" rather than "route somewhere arbitrary".
            {
                let storage = storage.clone();
                let routing = routing.clone();
                tauri::async_runtime::spawn(async move {
                    match routing.refresh_from(&storage).await {
                        Ok(count) => tracing::info!(roles = count, "role matrix loaded"),
                        Err(err) => {
                            tracing::error!(error = %err, "loading the role matrix failed")
                        }
                    }
                });
            }

            // Crash recovery (master plan S14.6, S16): runs left `Running` or
            // `AwaitingApproval` by a previous process become explicit,
            // resumable `Interrupted` runs with their unfinished attempts
            // closed. Never auto-resumes -- the user decides, from a state
            // that tells the truth about the crash instead of pretending
            // nothing happened.
            {
                let engine = engine.clone();
                tauri::async_runtime::spawn(async move {
                    match nacc_orchestrator::recovery::reconcile(&engine).await {
                        Ok(recovered) if recovered.is_empty() => {
                            tracing::info!("startup recovery found no interrupted runs");
                        }
                        Ok(recovered) => {
                            for run in recovered {
                                tracing::info!(
                                    run = %run.run_id,
                                    requeued_nodes = run.requeued_nodes,
                                    "startup recovery reconciled an interrupted run"
                                );
                            }
                        }
                        Err(err) => {
                            tracing::error!(error = %err, "startup recovery failed");
                        }
                    }
                });
            }

            // Sync the built-in template catalog (S24 Phase 7's "versioned
            // DAG templates"): the code is the upstream truth for presets,
            // storage is the catalog the GUI reads. A content change in a
            // build bumps the stored version; unchanged presets are left
            // exactly as they are.
            {
                let storage = storage.clone();
                tauri::async_runtime::spawn(async move {
                    for template in nacc_orchestrator::built_in_templates() {
                        let name = template.name.clone();
                        let existing = match storage.get_workflow_template(&name).await {
                            Ok(existing) => existing,
                            Err(err) => {
                                tracing::error!(
                                    template = %name,
                                    error = %err,
                                    "reading a stored template failed"
                                );
                                continue;
                            }
                        };
                        if let Some(current) = &existing {
                            if current.is_built_in
                                && current.description == template.description
                                && current.definition == template
                            {
                                continue;
                            }
                        }
                        let now = now_millis();
                        let (version, created_at_millis) = existing
                            .map(|current| (current.version + 1, current.created_at_millis))
                            .unwrap_or((1, now));
                        let record = nacc_storage::WorkflowTemplateRecord {
                            name,
                            description: template.description.clone(),
                            version,
                            is_built_in: true,
                            definition: template,
                            created_at_millis,
                            updated_at_millis: now,
                        };
                        if let Err(err) = storage.upsert_workflow_template(&record).await {
                            tracing::error!(
                                template = %record.name,
                                error = %err,
                                "syncing a built-in template failed"
                            );
                        }
                    }
                    tracing::info!("built-in workflow templates synced");
                });
            }

            app.manage(AppState {
                diagnostics_run_id,
                storage,
                providers,
                routing,
                engine,
                worktrees,
                run_leases: std::sync::Arc::new(std::sync::Mutex::new(
                    std::collections::HashMap::new(),
                )),
                _tracing_guard: guard,
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the NACC application");
}

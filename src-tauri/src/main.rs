// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// `--export-bindings`: generate `src/bindings.ts` (workspace root) and
/// exit, without starting the actual Tauri app (no window, no webview).
/// Used by CI (`.github/workflows/ci.yml`) to regenerate the gitignored
/// bindings file before the frontend build consumes it.
///
/// A separate `tests/export_bindings.rs` integration test originally did
/// this instead. That test binary consistently crashed on the CI runner
/// with `STATUS_ENTRYPOINT_NOT_FOUND` before even reaching its own code --
/// `dumpbin /dependents` showed a completely ordinary system-DLL import
/// table (no WebView2, nothing unusual), while this same workspace's
/// *other* test binaries (the lib's own unit tests, the main binary's own
/// empty test harness) ran cleanly in the same job. That pointed at
/// something specific to how a standalone `tests/` integration-test crate
/// gets linked against this particular dependency graph, not a missing
/// system DLL -- so rather than keep chasing an unexplained Windows
/// loader failure, this reuses the main binary target, which is already
/// proven to run cleanly.
fn main() {
    if std::env::args().any(|arg| arg == "--export-bindings") {
        let out_path = nacc_app_lib::bindings_output_path();
        nacc_app_lib::specta_builder()
            .export(specta_typescript::Typescript::default(), &out_path)
            .expect("failed to export TypeScript bindings");
        println!("wrote {}", out_path.display());
        return;
    }

    nacc_app_lib::run();
}

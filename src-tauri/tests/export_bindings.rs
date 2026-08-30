//! Regenerates `../src/bindings.ts` (the typed IPC surface the frontend
//! imports) as part of `cargo test`. `src/bindings.ts` is a generated
//! artifact -- gitignored, not committed -- so this test running before
//! the frontend build step is what actually produces it; see
//! `.github/workflows/ci.yml`'s step ordering (Rust steps before frontend
//! steps) and `docs/architecture/overview.md` for why, including the
//! honest limitation this creates for local frontend-only work on a
//! machine where the Rust side cannot currently link (see Phase 0's
//! foundation audit).

#[test]
fn export_typescript_bindings() {
    let builder = nacc_app_lib::specta_builder();

    builder
        .export(specta_typescript::Typescript::default(), "../src/bindings.ts")
        .expect("exporting TypeScript bindings should succeed");

    let generated =
        std::fs::read_to_string("../src/bindings.ts").expect("bindings.ts should exist immediately after export");

    assert!(
        generated.contains("get_app_diagnostics"),
        "generated bindings must reference every registered command; \
         get_app_diagnostics is missing from:\n{generated}"
    );
    assert!(
        !generated.trim().is_empty(),
        "generated bindings.ts must not be empty"
    );
}

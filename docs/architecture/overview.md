# Architecture overview — Phase 1–7

**Status:** reflects what is actually implemented through Phase 7's durable
DAG engine (committed; the engine is not yet wired into the application —
that is the next task). The authoritative, continuously updated per-crate
state, verification method, and remaining-work plan live in
[status.md](status.md) — read that file first when picking this repository
up. This document records the architecture itself; do not treat any
"Phase N scope" note below as already done.

## What Phase 1 delivers

Per the master plan's roadmap (§24) and build prompt (§17): current stable
Tauri 2 pinned and verified, Rust workspace boundaries, typed IPC, tracing
and error handling, strict capabilities/CSP, reproducible frontend and
desktop builds, and basic signed-updater development configuration. No
business logic, no real provider CLI invocation, no durable storage, no
workflow engine — those are later phases, listed per-crate below.

## What Phase 2 delivers

Per the master plan's roadmap (§24, §4.4): a real SQLite schema, embedded
versioned transactional migrations, and repository implementations for
exactly the data groups Phase 2's own deliverable list names -- settings,
role profiles, events, and audit records. §4.4 lists many more data groups
(provider installations, discovered models, workflow templates/runs,
worktree allocations, quality-gate results, review findings, CI/CD records,
approvals, policy decisions, usage estimates, updater state); those get
their own migration once their owning phase's crate has real logic to back
them, not schema'd speculatively now against a design those phases haven't
made yet -- the same scope discipline Phase 1 applied to every placeholder
crate, applied here to storage too.

Concretely, new in Phase 2:

- `nacc-domain`: `RoleKind` (the 18-role catalog from the Phase 0 plan's
  "locked GUI requirement" addendum, plus `Custom(String)` for
  user-defined roles) and `RoleProfile` (the four independently settable
  Role Matrix switches -- role, model, thinking, reasoning effort -- plus
  a permission profile), and four new strong IDs: `NodeRunId`, `AttemptId`,
  `EventId`, `AuditEventId`.
- `nacc-events`: `Event` (the closed, normalized event vocabulary from
  master plan §8.2 -- 20 variants, deliberately no `Other(String)` escape
  hatch) and `AuditRecord` (the audit-trail shape §22 requires: actor,
  action, requested vs. actual provider/model, effective
  reasoning/permission, command and redacted arguments, working
  directory). Pure domain types, no SQLite dependency.
- `nacc-storage`: `Database::open`/`open_in_memory`, real migrations
  (`migrations.rs`), and repository methods (`settings.rs`,
  `role_profiles.rs`, `events.rs`, `audit.rs`) -- every method that touches
  the connection is `async` and runs the actual (synchronous) SQLite call
  inside `tokio::task::spawn_blocking`, so a query never blocks the async
  runtime. `Database::backup_to`/`restore_from` use SQLite's own
  `VACUUM INTO` rather than a raw file copy, specifically because a raw
  copy of an open WAL-mode database can capture an inconsistent snapshot.
- `src-tauri`: `AppState` now holds a real `nacc_storage::Database`,
  opened once in `.setup()` at the real app-data directory.
  `get_app_diagnostics` (`diagnostics.rs`) became `async` and does a real
  storage round trip on every call -- writes one `AuditRecord`, reads back
  how many this session has recorded, and reports it alongside the applied
  schema version. Not `tauri::async_runtime::block_on`-ed from `.setup()`:
  see that function's own doc comment for why that specific pattern
  deadlocks/panics (`.setup()` already runs inside Tauri's own Tokio
  runtime) and why the round trip lives entirely inside one already-async
  command instead.

**rusqlite over sqlx**, verified via crates.io on 2026-08-30: rusqlite
0.40.2 (31.5M recent downloads, updated 2026-08-08) is a thin, synchronous
binding over the same `libsqlite3-sys` C bindings sqlx itself uses, with no
compile-time query-cache/build-time coupling -- sqlx's headline feature
needs a live database or a committed `.sqlx/` offline cache at build time,
exactly the kind of exotic build-time surface this workspace's own CI
history (`STATUS_ENTRYPOINT_NOT_FOUND`, the workspace-target-path incident
below) says to avoid. NACC is a single-process, single-user desktop app
with no concurrent-multi-client story sqlx's async pool is built for, so
synchronous rusqlite calls wrapped in `spawn_blocking` are the simpler,
equally-correct fit. `rusqlite_migration` (2.6.0, 2.2M recent downloads)
tracks the applied version in SQLite's own `user_version` pragma and
applies each pending migration transactionally -- matching master plan
§4.4's wording ("embedded, versioned, transactional") exactly. See the
root `Cargo.toml`'s own comment on these dependencies for the full
reasoning.

Every enum-typed SQL column (`ProviderId`, `ThinkingMode`, `ReasoningLevel`,
`PermissionProfile`, `RoleKind`, `EventType`, ...) is stored as that type's
own `serde_json` encoding rather than a hand-written second `Display`/
`FromStr` mapping -- one source of truth for a type's wire *and* storage
representation, instead of two that could silently drift apart. See
`nacc-storage/src/migrations.rs`'s own doc comment.

**Verified end-to-end, not just green-checkmarked**: this code could not
be compiled locally at all (no Windows SDK on this machine, confirmed the
hard way this session -- even `serde`'s own build script fails to link
here), so every claim above rests on real CI evidence, not local
`cargo check`. The first three pushes each failed clippy on a real,
distinct bug, fixed in sequence rather than guessed around in one shot:

1. `error[E0277]: the trait specta::Type is not implemented for
   serde_json::Value` (`Event.payload`) -- specta gates external-crate
   `Type` impls behind a feature flag per crate, same as Phase 1's `uuid`
   discovery; fixed by adding `serde_json` to the workspace's `specta`
   feature list, verified against specta's own `Cargo.toml` first.
2. `error[E0597]: 'conn'/'stmt' does not live long enough`, in all three
   `list_*` repository functions (`audit.rs`, `events.rs`,
   `role_profiles.rs`) -- `stmt.query_map(...)?.collect()` in tail
   position creates a `?`-operator temporary that Rust's drop-order rules
   extend past the block's own `conn`/`stmt` locals. Fixed identically in
   all three by binding the result to a local variable before returning
   it (the compiler's own suggested fix).
3. `error[E0004]: non-exhaustive patterns:
   rusqlite_migration::SchemaVersion::Outside(_) not covered` in
   `diagnostics.rs` -- `SchemaVersion` has three variants (`NoneSet`,
   `Inside`, `Outside`), not the two an earlier docs.rs summary reported;
   fixed with real display text for `Outside` (a genuine state: the
   database's `user_version` is higher than any migration this build's
   `Migrations` list knows about), not a stub.

Run [33330656904](https://github.com/jerryboganda/NACC/actions/runs/33330656904)
(SHA `038a259`) passed all 21 steps -- `cargo fmt`, `cargo clippy
--workspace --all-targets --all-features -D warnings`, `cargo test
--workspace` (every test in this section's new code actually ran and
passed: migration upgrade-path tests, settings/role-profile/event/audit
repository tests, the backup/restore round trip), bindings regeneration,
the frontend build type-checking against those real bindings, Vitest, and
a real signed `tauri build` -- confirmed via
`gh api repos/jerryboganda/NACC/actions/runs/.../artifacts` returning a
genuine, non-expired, SHA256-digested installer artifact
(`nacc-windows-installer-038a259...`, 4,224,969 bytes), not just a green
step.

## What Phases 3–7 deliver

- **Phase 3 — process, runtime, worktrees.** `nacc-process` contains
  process trees in Windows Job Objects (kill-on-close, so cancellation
  leaves no orphans); `nacc-git` is a typed CLI wrapper (argument arrays,
  never interpolated shell); `nacc-worktree` implements the lease
  lifecycle (allocate, inspect, release, quarantine, reconcile);
  `nacc-runtime` probes for installed tooling. PTY/ConPTY is deliberately
  deferred — no adapter's non-interactive launch mode needs one, and
  `CapabilitySnapshot.interactive_pty` reports the absence as a fact.
- **Phase 4 — provider framework.** `nacc-provider-core`: the
  `AgentProvider` trait, `CapabilitySnapshot`, health ranking, registry,
  the normalized `ProviderEvent` vocabulary, and a 9-check contract suite
  proven able to fail.
- **Phase 5 — Claude and Codex adapters.** Real permission/reasoning
  mapping, launch/resume argv, stream parsers, honest usage reporting —
  but execution is fixture-only and **no adapter is wired outside its own
  crate yet**; live CLI launch is not verified.
- **Phase 7 — durable DAG engine.** `nacc-orchestrator`: a pure scheduler,
  a concurrency governor, an injectable clock, the checkpoint-per-
  transition `WorkflowEngine`, startup recovery that never auto-resumes,
  and four built-in templates as real DAGs; `nacc-storage` migration V5
  adds the workflow-state tables and repository. The engine is fully
  tested against a real in-memory database, but nothing in `src-tauri`
  can start a run yet — wiring it into the application is the next task
  (status.md, Task D).

## Toolchain, pinned and verified (not assumed)

| | Version | Verified |
|---|---|---|
| Rust | 1.96.0 (`rust-toolchain.toml`) | Ahead of two independent floors: Tauri 2.12's declared MSRV bump (1.77.2 → 1.90) and specta 2.0.0-rc.25's use of `std::fmt::from_fn`, stable only since Rust 1.93.0. The lower bound (1.90.0) was tried first and genuinely failed CI with `error[E0658]: use of unstable library feature` inside specta's own source — a real rustc requirement, not a clippy-only lint, caught by this workspace's own CI run rather than assumed. |
| `tauri` | 2.11.5 | crates.io, 2026-08-30 |
| `tauri-build` | 2.6.3 | crates.io, 2026-08-30 — default features (`config-json`) required since `tauri.conf.json` is JSON, not JSON5/TOML |
| `tauri-plugin-updater` | 2.10.1 | crates.io, 2026-08-30 |
| `specta` / `tauri-specta` / `specta-typescript` | `=2.0.0-rc.25` / `2.0.0-rc.25` / `0.0.12` | crates.io, exact-pinned per the crate's own doc comment warning ("During the beta period, it is really important you use `=`"). Genuinely still release-candidate versioned after 2+ years, but with 547K/90-day downloads on `tauri-specta` alone — de facto production standard in the Tauri 2 ecosystem, not an experimental choice. See the ADR-0001-style reasoning captured in this session's Phase 1 work log if this needs revisiting. |
| React / Vite / TypeScript / Vitest | 19.2.8 / 8.2.2 / 7.0.2 / 4.1.11 | npm registry, 2026-08-30 |

## Workspace structure

```
Cargo.toml                 # workspace root, resolver = "2", 21 crates + src-tauri
rust-toolchain.toml        # pinned 1.96.0
src-tauri/                 # Tauri 2 application shell (composition root)
crates/
  nacc-domain               # REAL: strong IDs, PermissionProfile (with rank/narrower_of),
                             # RoleKind/RoleProfile, workflow state machine types (Phases 2, 7)
  nacc-events                # REAL (Phase 2): Event, AuditRecord -- normalized vocabulary and
                             # audit-trail shape, no SQLite dependency
  nacc-observability         # REAL: init_tracing(), correlation-span helper
  nacc-storage               # REAL (Phases 2, 7): SQLite schema V1-V5, migrations,
                             # settings/role-profile/event/audit/worktree/provider/workflow
                             # repositories
  nacc-process               # REAL (Phase 3): Windows Job Object process-tree containment
  nacc-git                   # REAL (Phase 3): typed git CLI wrapper
  nacc-worktree              # REAL (Phase 3): worktree lease lifecycle
  nacc-runtime               # REAL (Phase 3, detection): tooling probes
  nacc-provider-core         # REAL (Phase 4): AgentProvider trait, capability/event types,
                             # registry, health, 9-check contract suite (master plan S8)
  nacc-provider-claude       # REAL (Phase 5, fixture-only execution; not wired into the app)
  nacc-provider-codex        # REAL (Phase 5, fixture-only execution; not wired into the app)
  nacc-provider-antigravity  # STUB (Phase 8): blocked on a real headless interface
  nacc-provider-copilot      # STUB (Phase 5/10): CLI contract recorded, not implemented
  nacc-provider-opencode     # STUB (Phase 8): blocked on a working local binary
  nacc-orchestrator          # REAL (Phase 7): durable DAG engine -- scheduler, governor,
                             # clock, engine, recovery, templates; not yet wired into the app
  nacc-github, nacc-policy, nacc-quality, nacc-review, nacc-secrets,
  nacc-updater               # Boundary + typed error only. Each crate's own doc comment
                             # names its target phase (9, 10, 11, or 12).
src/                        # React + TypeScript + Vite frontend
  App.tsx                    # the one real IPC round trip, now a real storage round trip too
                             # (see below)
  bindings.ts                # GENERATED, gitignored -- see "Typed IPC" below
```

Every crate depends only on `nacc-domain` (or, for provider crates,
`nacc-provider-core`, which itself depends only on `nacc-domain`) plus
whatever external crates it genuinely uses. `src-tauri` is the only crate
that depends on all of them — the composition root, matching master plan
§6's dependency-direction rule ("provider crates depend on a common
provider core... the orchestrator depends on abstractions, not concrete
CLI parsers").

## Typed IPC: the one real end-to-end round trip

`get_app_diagnostics` (`src-tauri/src/diagnostics.rs`) is Phase 1's proof
that the whole pipe works, not a real feature:

```
Rust command (#[tauri::command] #[specta::specta])
  -> tauri-specta Builder.export() generates ../src/bindings.ts
  -> React calls commands.getAppDiagnostics()
  -> renders AppDiagnostics, including a real nacc_domain::WorkflowRunId
```

**`src/bindings.ts` is generated, not committed** (`.gitignore`). It is
produced by `cargo run -p nacc-app -- --export-bindings` (see
`src-tauri/src/main.rs`, which calls the exact same `specta_builder()`
construction the running app itself uses) and consumed by the frontend's
`tsc`/`vite build` step immediately after. Hand-transcribing tauri-specta's
~1000-line codegen template accurately enough to commit a byte-correct copy
was assessed and rejected as higher-risk than regenerating fresh — see this
session's Phase 1 work log for the specific mismatch this avoided
(`specta`'s built-in `SystemTime` mapping does not match a custom
`serde(with)` wire format, which is exactly the kind of silent drift a
committed, hand-maintained bindings file invites).

Generation originally ran as a `tests/export_bindings.rs` integration
test invoked via `cargo test`, which is the more common pattern in the
tauri-specta ecosystem. That integration-test binary consistently crashed
on this workspace's CI runner with `STATUS_ENTRYPOINT_NOT_FOUND` before
reaching any of its own code, while every *other* test binary in the same
job (the lib's own unit tests, the main binary's own empty test harness)
ran cleanly — `dumpbin /dependents` showed a completely ordinary system-DLL
import table, ruling out an obvious missing-DLL cause. Rather than keep
chasing an unexplained low-level Windows loader failure specific to
standalone integration-test crates, generation was moved into the main
`nacc-app` binary itself via a `--export-bindings` flag that exits before
starting the actual Tauri app (no window, no webview) — reusing a binary
target already proven to run cleanly on the same runner.

**Known limitation, stated plainly**: on a machine where the Rust side
cannot currently link (see the Phase 0 foundation audit — no Windows SDK
installed, by policy; all Rust builds run in GitHub Actions), `src/
bindings.ts` cannot be regenerated locally. Frontend-only local iteration
uses a hand-written stub matching the same shape (see this session's Phase
1 work), never committed, always superseded by CI's real generated file.

## Tracing and error handling

`nacc-observability::init_tracing(log_dir, dev_mode)` configures two
layers sharing one `NACC_LOG`-controlled filter: a daily-rotating JSON file
(master plan §22's structured-log requirement) and a human-readable
console layer. Called once in `src-tauri`'s `.setup()`; the returned
`WorkerGuard` lives in `AppState` for the app's lifetime.

Every library crate follows master plan §4.2's rule ("typed errors in
libraries"): a `thiserror`-based `<Crate>Error` enum plus a `Result<T>`
alias. Crates without real logic yet have a single `Other(String)`
variant, documented as a placeholder that grows more specific arms as real
logic lands — not a permanent design.

`nacc_provider_core::ProviderError` additionally has an
`IneligibleCredential` variant added directly from Phase 0's live
findings: Copilot's ACP mode rejected a valid `gh` classic PAT with a
precise "not supported in this mode" error, and Gemini CLI rejected a
valid-but-ineligible account tier. Neither is "no credential" — both are
"this credential doesn't authorize this operation," matching ADR-0002's
recommendation to treat this as its own first-class case.

## Security configuration

- **CSP is set**, not `null` — the specific gap the Phase 0 audit found in
  upstream AgentPanel. See `src-tauri/tauri.conf.json`'s `app.security.csp`.
- **Capabilities are minimal**: `src-tauri/capabilities/default.json`
  grants only `core:default` and `updater:default`. No `fs:*`, `shell:*`,
  `http:*`, dialog, clipboard, or notification permissions — none are
  needed yet, and master plan §12/§13.1 require the webview never receive
  a general shell or filesystem capability regardless.
- **Updater has NACC's own signing key**, not upstream's. Generated this
  session via `npx tauri signer generate` (works without a Rust build —
  verified). The private key was stored directly as GitHub Actions
  secrets (`TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
  on `jerryboganda/NACC`) and never written to the repository or left on
  disk after. The public key is in `tauri.conf.json`. This is "basic...
  development configuration" per the master plan's own Phase 1 scoping —
  production key rotation and a real release channel are Phase 12.

## CI

Two workflows, deliberately distinct:

- `.github/workflows/foundation-audit.yml` — Phase 0 evidence. Builds
  **upstream** AgentPanel at a pinned SHA, not this repository's code. Kept
  for provenance; not part of ongoing CI.
- `.github/workflows/ci.yml` — ongoing CI for NACC's own code. Runs on
  every push to `main`: `cargo fmt --check` → `cargo clippy -D warnings`
  (workspace-wide) → `cargo test --workspace` → `cargo run -- --export-bindings`
  (regenerates `src/bindings.ts`) → frontend build/lint/typecheck → Vitest →
  a real `tauri build` producing a genuinely signed NSIS installer, uploaded as an
  artifact. Windows-only, per standing directive. Rust steps run before
  frontend steps deliberately — see "Typed IPC" above for why the ordering
  is load-bearing, not incidental.

  **Verified end-to-end, not just green-checkmarked**: run
  [33325051099](https://github.com/jerryboganda/NACC/actions/runs/33325051099)
  (SHA `4f2a148`) passed all 20 steps, but the first pass through the full
  pipeline surfaced a real bug the checkmark alone hid — the installer-locate
  and installer-upload steps looked for the NSIS bundle at
  `src-tauri/target/release/bundle`, which is where upstream AgentPanel's
  standalone single-crate layout puts it, but NACC is a Cargo *workspace*, so
  the real output is at the workspace-root `target/release/bundle`. Because
  the upload step used `if-no-files-found: warn`, the step stayed green while
  uploading nothing — only caught by querying
  `gh api repos/jerryboganda/NACC/actions/runs/<id>/artifacts` after the
  "successful" run and finding `total_count: 0`. Fixed by correcting both
  paths and changing `if-no-files-found` to `error` so this class of bug
  fails the build instead of hiding inside a passing step. See ci.yml's own
  header comment for the full account.

Not automated: `master plan §18`'s "Tauri development smoke launch" (i.e.
running `tauri dev` interactively) has no practical CI equivalent — a
production `tauri build` is stricter in most respects and is what CI
verifies instead. A real interactive smoke launch is a local-machine or
later-phase manual verification step.

## What is explicitly not here yet

The workflow engine exists and is fully tested, but **nothing can start a
run yet**: `src-tauri` has no orchestrator handle and no commands beyond
`get_app_diagnostics`, and the engine's `NodeExecutor`/`RoleRouting`
boundaries have no production implementations — nothing launches a real
provider CLI (the Phase 5 adapters are themselves not wired outside their
crates). Per-node timeouts and attempt leases/heartbeats are also not
built. Beyond that: no GUI beyond the one diagnostics screen (no Setup
Wizard, no Role Matrix UI — even though `RoleProfile` has real, tested
storage underneath it), no policy enforcement, no quality gates or review
findings, no real GitHub/Copilot CI integration, no secrets storage, no
updater logic. Every one of those has a crate boundary and a named target
phase; [status.md](status.md) §9 is the ordered plan for landing them.

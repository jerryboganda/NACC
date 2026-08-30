# Architecture overview — Phase 1

**Status:** reflects what is actually implemented as of Phase 1
("hardened Tauri/Rust modular foundation"). Updated as later phases land
real logic; do not treat any "Phase N scope" note below as already done.

## What Phase 1 delivers

Per the master plan's roadmap (§24) and build prompt (§17): current stable
Tauri 2 pinned and verified, Rust workspace boundaries, typed IPC, tracing
and error handling, strict capabilities/CSP, reproducible frontend and
desktop builds, and basic signed-updater development configuration. No
business logic, no real provider CLI invocation, no durable storage, no
workflow engine — those are later phases, listed per-crate below.

## Toolchain, pinned and verified (not assumed)

| | Version | Verified |
|---|---|---|
| Rust | 1.90.0 (`rust-toolchain.toml`) | Ahead of Tauri 2.12's declared MSRV bump (1.77.2 → 1.90); this workspace's own CI has already built successfully at this pin (Phase 0's `foundation-audit.yml` run) |
| `tauri` | 2.11.5 | crates.io, 2026-08-30 |
| `tauri-build` | 2.6.3 | crates.io, 2026-08-30 — default features (`config-json`) required since `tauri.conf.json` is JSON, not JSON5/TOML |
| `tauri-plugin-updater` | 2.10.1 | crates.io, 2026-08-30 |
| `specta` / `tauri-specta` / `specta-typescript` | `=2.0.0-rc.25` / `2.0.0-rc.25` / `0.0.12` | crates.io, exact-pinned per the crate's own doc comment warning ("During the beta period, it is really important you use `=`"). Genuinely still release-candidate versioned after 2+ years, but with 547K/90-day downloads on `tauri-specta` alone — de facto production standard in the Tauri 2 ecosystem, not an experimental choice. See the ADR-0001-style reasoning captured in this session's Phase 1 work log if this needs revisiting. |
| React / Vite / TypeScript / Vitest | 19.2.8 / 8.2.2 / 7.0.2 / 4.1.11 | npm registry, 2026-08-30 |

## Workspace structure

```
Cargo.toml                 # workspace root, resolver = "2", 21 crates + src-tauri
rust-toolchain.toml        # pinned 1.90.0
src-tauri/                 # Tauri 2 application shell (composition root)
crates/
  nacc-domain               # REAL: strong IDs, ReasoningLevel/ThinkingMode/PermissionProfile
  nacc-provider-core         # REAL: AgentProvider trait, capability/event types (master plan S8)
  nacc-provider-{claude,codex,antigravity,copilot,opencode}
                             # REAL trait impl, all methods return "not yet implemented" --
                             # proves the contract is satisfiable; CLI invocation is Phase 5/8
  nacc-observability         # REAL: init_tracing(), correlation-span helper
  nacc-storage, nacc-events, nacc-orchestrator, nacc-process, nacc-runtime,
  nacc-worktree, nacc-git, nacc-github, nacc-policy, nacc-quality,
  nacc-review, nacc-secrets, nacc-updater
                             # Boundary + typed error only. Each crate's own doc comment
                             # names its target phase (2, 3, 7, 9, 10, 11, or 12).
src/                        # React + TypeScript + Vite frontend
  App.tsx                    # the one real IPC round trip (see below)
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
produced by `cargo test -p nacc-app` (specifically
`src-tauri/tests/export_bindings.rs`, which calls the exact same
`specta_builder()` construction the running app uses) and consumed by the
frontend's `tsc`/`vite build` step immediately after. This is deliberate,
not an oversight: hand-transcribing tauri-specta's ~1000-line codegen
template accurately enough to commit a byte-correct copy was assessed as
higher-risk than making the frontend build depend on a fresh Rust-generated
file — see this session's Phase 1 work log for the specific mismatch this
avoided (`specta`'s built-in `SystemTime` mapping does not match a custom
`serde(with)` wire format, which is exactly the kind of silent drift a
committed, hand-maintained bindings file invites).

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
  (workspace-wide) → `cargo test --workspace` (which also regenerates
  `src/bindings.ts`) → frontend build/lint/typecheck → Vitest → a real
  `tauri build` producing a genuinely signed NSIS installer, uploaded as an
  artifact. Windows-only, per standing directive. Rust steps run before
  frontend steps deliberately — see "Typed IPC" above for why the ordering
  is load-bearing, not incidental.

Not automated: `master plan §18`'s "Tauri development smoke launch" (i.e.
running `tauri dev` interactively) has no practical CI equivalent — a
production `tauri build` is stricter in most respects and is what CI
verifies instead. A real interactive smoke launch is a local-machine or
later-phase manual verification step.

## What is explicitly not here yet

No SQLite, no migrations, no workflow engine, no policy enforcement, no
Windows Job Objects, no real Git/GitHub operations, no real provider CLI
invocation, no GUI beyond the one diagnostics screen. Every one of those
has a crate boundary and a named target phase already (see the workspace
structure table above) — Phase 1's job was the foundation those land on,
not the features themselves.

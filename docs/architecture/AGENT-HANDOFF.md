# NACC AGENT HANDOFF — read this fully before touching anything

**Date:** 2026-09-17. **Repo:** `D:\Projects\NACC` (branch `main`, local commits only — nothing pushed).
**Mission:** NACC (Native Agent Control Center) — a Windows-first, local-first Tauri 2 + Rust desktop GUI that orchestrates multiple native coding-agent CLIs (Claude Code, Codex, Antigravity, Copilot, OpenCode) with durable workflow runs, Role Matrix configuration, worktrees, quality gates, and approval-gated CI/CD. It is NOT an LLM and must never become a Node.js/Python/Go/.NET app server.

---

## 1. Binding specification (non-negotiable)

Two documents bind every decision. Read both before changing anything:

1. `native-agent-control-center-tauri2-rust-master-plan.md` (2090 lines) — governing product/architecture spec. §24 = phase roadmap, §27 = the 40 acceptance criteria, §20 = completion evidence (20-step E2E + extra demonstrations), §32 = conflict rule.
2. `native-agent-control-center-tauri2-rust-build-prompt.md` (688 lines) — execution prompt. §2 = technology constraints, §17 = phase sequence, §21 = operational behavior, §22 = final report format (never claim "100% bug-free" or complete support where evidence is absent).

**Conflict rule:** a verified platform limitation conflicts with the plan → document it in an ADR, preserve the intent, choose the safest maintainable implementation. Never silently weaken a requirement.

**Hard constraints (§2):** Tauri 2 (never Electron); privileged backend in Rust; React+TS+Vite frontend; no runtime Node server; all process execution/PTY/persistence/Git/policy/secrets/audit in Rust; webview never gets a shell; SQLite + embedded Rust migrations; Windows Job Object containment; separate worktrees for write workers; GitHub Actions stays the deterministic CI/CD; production deploy/destructive ops/secret changes stay approval-gated; provider OAuth tokens are never copied or read (only presence-checked); no hard-coded model marketing names (discover/validate at runtime); never pretend a provider setting was applied (disable unsupported controls and say why).

**Operational rules (§21):** no push, no releases, no repo-visibility changes, no production deploy unless explicitly authorized; small reviewable commits by phase; preserve user work; verify, don't assume; honest degraded modes; fix root causes.

---

## 2. Environment reality (this machine) — read before building

- **Default MSVC toolchain cannot link.** VS Build Tools are installed but the Windows SDK import libs are missing, so every link fails (`link.exe not found`, exit 101). Do not try to fix by installing things; use the GNU toolchain instead.
- **Working toolchain:** `rustup toolchain list` shows `stable-x86_64-pc-windows-gnu (default)`, `1.96.0-x86_64-pc-windows-gnu`, and `1.96.0-x86_64-pc-windows-msvc (active)`. `rust-toolchain.toml` pins 1.96.0. The **GNU override beats rust-toolchain.toml** and links via scoop's gcc (`C:\Users\Dr Faisal Maqsood PC\scoop\apps\gcc\current\bin\gcc.exe`).
- **Every long Rust command must run in the background.** The agent shell tool kills foreground commands at ~30 s; a clean debug build of `nacc-app` takes ~90 s+ (first build ~3 min). Pattern that works (PowerShell):
  ```powershell
  $p = Start-Process -FilePath (Get-Command cargo).Source `
    -ArgumentList '+1.96.0-x86_64-pc-windows-gnu','test','-p','nacc-app' `
    -WorkingDirectory 'D:\Projects\NACC' `
    -RedirectStandardOutput 'D:\Projects\NACC\output\x.stdout.log' `
    -RedirectStandardError  'D:\Projects\NACC\output\x.stderr.log' -PassThru
  # poll: Get-Process -Id $p.Id; then read the logs
  ```
  For multi-step scripts use `Start-Process powershell.exe -EncodedCommand <base64>`.
- **Never kill a running cargo/rustc to "retry".** Two long builds that looked hung both finished successfully (`Finished dev profile ... in 1m31s`, `in 3m 00s`). Check the log tail + process CPU before concluding anything.
- **PowerShell quirk:** `cargo ... 2>&1 | Select-Object` turns native stderr into a `NativeCommandError` and hides real output. Redirect to a file (`2> file`) or use the Start-Process pattern instead.
- `output/` is gitignored — scratch logs live there. `target/`, `node_modules/`, `.freebuff/`, `src/bindings.ts` (generated) are gitignored too.
- Frontend: Node 24, `npm.cmd` on PATH; `npm run build` = `tsc && vite build`; tests = `vitest run` (jsdom, setup in `src/test/setup.ts`). Frontend checks are fast (<10 s) and can run in the foreground.

---

## 3. What is DONE and verified (commits, oldest → newest)

All green CI history before this session is recorded in `docs/architecture/status.md` (§6/§12): Phases 0–5 complete and CI-verified (`db2588d` era), last green CI run `35164045996` with a real signed installer artifact.

This session's work (all local commits, **not pushed**):

1. **`2403554` — Phase 6 (part 1): Role Matrix GUI over role-profile IPC.**
   Finished the interrupted `RoleProfileView` conversion (create/update return the view with string timestamps); added `src/RoleMatrix.tsx` + `RoleMatrix.css` + `RoleMatrix.test.tsx`; mounted in `App.tsx`. CRUD over the generated commands: create/edit, custom roles (`{custom: string}`), independent provider/model assignment, enable/disable, confirm-before-delete, loading/error/retry, duplicate-submit + stale-response protection (versioned ref guard). Thinking/reasoning selectors deliberately DISABLED with a "capability discovery not connected" explanation (S10.1 honesty rule). Verified: GNU `cargo check -p nacc-app`, GNU `cargo test -p nacc-storage --lib` (48 passed), `npm run build`, `npm test` (11 passed).
2. **`757cb3d` — Phase 4 GUI slice: provider detection + GNU test-manifest fix.**
   - `src-tauri/src/providers.rs`: `build_registry()` (real Claude Code + Codex adapters over `ProcessCommandRunner`), `detect_provider` (runs the CLI's real `--version` through the contained supervisor, 15 s timeout, persists a `ProviderInstallation`), `list_provider_installations`, `check_provider_auth` (live credential-store *presence* check — contents never read, S8.4). Views stringify timestamps (same convention as RoleProfileView).
   - `src/Providers.tsx`: Providers panel — Detect buttons (explicit, click-only, never auto-run), Check sign-in buttons, honest copy: observations ≠ authentication ≠ readiness; command names ≠ resolved absolute paths.
   - **ROOT-CAUSED + FIXED the `0xc0000139` (STATUS_ENTRYPOINT_NOT_FOUND) local test failure** in `src-tauri/build.rs`: the app binary embeds Tauri's Common Controls v6 manifest via Tauri's `cargo:rustc-link-arg-bins=...libresource.a`, which only reaches *binary* targets — the lib test harness (which imports `TaskDialogIndirect` from comctl32.dll) got no manifest and died before `main`. Diagnosis path that worked: run the test exe directly → `dumpbin /dependents` (rules out ICU/VC++ redistributables) → `objdump -p` import table (`TaskDialogIndirect` present, no `.rsrc` section) → controlled experiment linking Tauri's generated `OUT_DIR/libresource.a` into the test → passes. Fix (GNU-only guard): `println!("cargo:rustc-link-arg={}", resource.display())`. Verified: plain `cargo +1.96.0-x86_64-pc-windows-gnu test -p nacc-app` passes 6/6; `run -p nacc-app -- --export-bindings` still works; clippy `-D warnings` clean.
   - **TRAP:** `cargo:rustc-link-arg-tests` is NOT a valid instruction in this Cargo version — the whole build fails with `invalid instruction`. Never use it.
3. **UNCOMMITTED (in flight, see §4):** engine wiring — `executor.rs`, `routing.rs`, `workflows.rs`, `lib.rs` AppState/setup wiring, `providers.rs` auth command, `Providers.tsx` auth UI. Compilation was last verified BEFORE the final edits; the uncommitted state has NOT been compiled since.

---

## 4. THE IN-FLIGHT SLICE — exact state and immediate next steps

Uncommitted files (`git status`): modified `Cargo.lock`, `src-tauri/Cargo.toml` (+`tokio`, +`async-trait` deps), `src-tauri/src/lib.rs`, `src-tauri/src/providers.rs`, `src/Providers.tsx`; **untracked** `src-tauri/src/executor.rs`, `src-tauri/src/routing.rs`, `src-tauri/src/workflows.rs`.

What each contains:

- **`routing.rs`** — `RoleMatrixRouting`: engine `RoleRouting` over persisted `RoleProfile`s (enabled rows only; first-enabled-wins), plus `settings_for` (reasoning/thinking from the same row), `set_workspace`/`workspace_for_project` (per-project explicit workspace), `validate_workspace` (absolute + existing dir or refuse), `refresh_from(&Database)` (reload snapshot), `routable_role_count`. 6 unit tests included. The `RoleRouting` impl maps provider/model/permission/workspace.
- **`executor.rs`** — `ProviderNodeExecutor` implementing `nacc_orchestrator::NodeExecutor` for real: refuses (permanent failure) when a role has no provider/model or a project has no workspace (never guesses); resolves settings from the shared routing snapshot; builds `ResolvedAgentProfile` (NativeWindows runtime); `launch`es through `dyn AgentProvider` with a channel-backed `EventSink`; awaits exactly one terminal event (`SessionCompleted`/`SessionCancelled`/`TerminalError`) with a 30-min default timeout (`DEFAULT_NODE_TIMEOUT`, `with_timeout`); on timeout cancels via `provider.cancel(session, Forced)` so the Job Object tree dies; persists every normalized event durably via `nacc_events::Event` + `storage.append_event` with full correlation IDs (project/run/node_run/attempt), best-effort with tracing on failure; maps terminal events to `NodeExecutionOutcome` (bounded 4000-char assistant-text summary, explicit `…` marker) or `NodeExecutionFailure` (NotInstalled→permanent, other launch errors→retryable, cancel→permanent, terminal error→retryable). `event_type_for` is an exhaustive match so new `ProviderEvent` variants are a compile error, not silent data loss.
- **`workflows.rs`** — Run Console IPC: `list_workflow_templates`, `start_workflow_run` (validates workspace, sets it on routing, `engine.start_run`, then `drive_in_background` so the command returns immediately), `get_workflow_run`, `list_workflow_runs` (uses `ALL_STATES`), `pause_workflow_run`, `cancel_workflow_run`, `resume_workflow_run` (refuses `AwaitingApproval` with a precise message; drives in background), `decide_workflow_approval` (approve/reject with required `by`; reject requires reason; optional `resume_after`), `list_workflow_events`. View types (`RunSnapshotView` etc.) stringify timestamps; `snapshot_view` maps `RunSnapshot`. 3 unit tests.
- **`lib.rs`** — `AppState` gained `providers: Arc<ProviderRegistry>`, `routing: Arc<RoleMatrixRouting>`, `engine: Arc<WorkflowEngine>`; `.setup()` builds the object graph (executor shares routing with the engine), spawns an async `routing.refresh_from` at startup (empty snapshot until then = "nothing routable", never "route anywhere"), and manages state. `mod` declarations for executor/routing/workflows added.
- **`providers.rs` / `Providers.tsx`** — the auth slice on top of the committed milestone (detect+list were committed; uncommitted adds `check_provider_auth` + Check sign-in UI with an `authChecking` ref guard and live status lines).

**IMMEDIATE NEXT STEPS (do these first, in order):**

1. Register the workflow commands in `specta_builder()` (`src-tauri/src/lib.rs`): add `workflows::list_workflow_templates`, `workflows::start_workflow_run`, `workflows::get_workflow_run`, `workflows::list_workflow_runs`, `workflows::pause_workflow_run`, `workflows::cancel_workflow_run`, `workflows::resume_workflow_run`, `workflows::decide_workflow_approval`, `workflows::list_workflow_events` to `collect_commands![]`. NOT done yet — without this the commands do not exist over IPC and bindings will not include them.
2. Refresh the routing snapshot after every role-profile mutation: in `src-tauri/src/role_profiles.rs`, after successful create/update/set_enabled/delete, call `state.routing.refresh_from(&state.storage).await` (log failures; the snapshot must never go stale after a GUI edit). NOT done yet.
3. Compile + fix errors: `cargo +1.96.0-x86_64-pc-windows-gnu check -p nacc-app` (background pattern from §2). `nacc-events` and `nacc-orchestrator` are already deps of nacc-app (Cargo.toml lines ~46–50). Known risk: `specta::Type` on views requires every field type to impl it — `RunState`/`NodeState`/`EventType` do; `ApprovalId`/`NodeRunId` do.
4. `cargo fmt --all`, then GNU `test -p nacc-app` (expect 6 existing app + 3 workflows + 6 routing + 2 providers tests), then `run -p nacc-app -- --export-bindings` to regenerate `src/bindings.ts`, then assert `startWorkflowRun`/`listWorkflowTemplates`/`decideWorkflowApproval`/`listWorkflowEvents` are present in the generated file.
5. Frontend: `npm run build` + `npm test`. Add the new commands to the `vi.mock("./bindings", ...)` factory in `src/App.test.tsx` (currently mocks `getAppDiagnostics`, `listRoleProfiles`, `listProviderInstallations` — a missing key makes App tests fail once App renders new panels).
6. Build the Run Console page (`src/RunConsole.tsx`): list templates → start run (project id + workspace path inputs; reuse RoleMatrix's table/CSS patterns, `role-matrix` classes already exist), runs list with states, run detail (nodes/attempts/checkpoints/approvals), pause/cancel/resume with reasons, approval decide UI (by + reason; approve+resume), events list (`payload_json` shown verbatim). Tests mock the IPC boundary like `RoleMatrix.test.tsx` does. Mount in `App.tsx`.
7. Update `docs/architecture/status.md` (new checkpoint section on top; honest about what is verified vs not), then commit locally, e.g. `Phase 7 wiring (Task D): engine into the app + Run Console IPC`.
8. Only then consider pushing/CI — see §8 for what still needs real CI.

---

## 5. Verification commands (exact)

```powershell
# Rust (background pattern; logs in output\)
cargo +1.96.0-x86_64-pc-windows-gnu check -p nacc-app
cargo +1.96.0-x86_64-pc-windows-gnu test -p nacc-app
cargo +1.96.0-x86_64-pc-windows-gnu test -p nacc-storage --lib
cargo +1.96.0-x86_64-pc-windows-gnu test -p nacc-provider-claude -p nacc-provider-codex --lib
cargo +1.96.0-x86_64-pc-windows-gnu clippy -p nacc-app --all-targets -- -D warnings
cargo fmt --all -- --check
cargo +1.96.0-x86_64-pc-windows-gnu run -p nacc-app -- --export-bindings   # regenerates src/bindings.ts

# Frontend (foreground OK)
npm.cmd run build
npm.cmd test
```

Expected current test counts: nacc-app 6 (grows with workflows/routing), storage 48, claude 25, codex 19, frontend 15 (grows with RunConsole). Storage is ~1 s warm; adapters ~12 s cold compile; anything cold needs the background pattern.

---

## 6. Traps that already cost time — do not repeat them

1. **`src/bindings.ts` is generated, never hand-written.** A stale handwritten stub once broke the frontend build. Always regenerate via the export command after changing commands/types.
2. **`cargo:rustc-link-arg-tests` is invalid** in this Cargo — the whole build fails. Use `cargo:rustc-link-arg` (duplicate static inclusion into bins is harmless; verified).
3. **Editor/replace mistakes on long files**: two edits in this session dropped lines by replacing a block whose tail was not included in `old_text`. Rule: for files >150 lines, read the target region first and include the entire block (through its closing braces) in BOTH old_text and new_text.
4. **Don't `unwrap_err()` on types without Debug** (e.g. `&Arc<dyn AgentProvider>`) — match on the Err instead (see registry tests).
5. **Tauri MockRuntime integration tests** now work thanks to the build.rs manifest fix; if a test exe still dies with `0xc0000139`, check that `libresource.a` exists in the CURRENT build dir (`target/debug/build/nacc-app-*/out/` paths are per-fingerprint).
6. **Agent teammates (sub-agent spawning) failed with auth errors** in this environment; do all work directly.
7. **`ProviderEvent` → durable events**: every variant must be mapped in `event_type_for`; the exhaustive match makes additions a compile error. Keep it that way.
8. **Timestamps across IPC are strings** (RoleProfileView, ProviderInstallationView, workflows views). Keep the convention — the webview never formats a bare epoch.
9. **Long-file edit discipline**: when an edit fails with "text not found" or lands in the wrong place, STOP and re-read the file before the next edit; never stack a second blind edit on a corrupted file.

---

## 7. Remaining work after the in-flight slice (ordered)

**Task D remainder (Phase 7 wiring):** per-node timeouts are a global default only (master plan wants node-declared timeouts); cancel of a run does not yet abort in-flight sessions (engine `cancel` marks state; the executor's sessions keep running until they end or time out — wire run→session cancellation); worktree lease allocation is NOT wired (runs use the user-chosen workspace; `nacc-worktree` exists with real leases + tests); recovery at startup (`nacc_orchestrator::recovery` / `mark_interrupted` / `requeue_unfinished`) is not yet invoked from `.setup()`; template persistence (templates are code presets, not versioned DB rows yet); the two missing §18 presets.

**Phase 6 remainder:** Setup Wizard (S17.1's 12 steps), capability snapshot wiring so thinking/reasoning selectors enable honestly (adapters' `capabilities()` exist; needs an IPC command + storage snapshots), multiple accounts per provider, fallback chains in the Role Matrix UI.

**Phases 8–12 (not started):** Antigravity + OpenCode adapters (crates exist, contract-stage); review/quality system (`nacc-review`, `nacc-quality` contract-stage); GitHub/Copilot CI/CD integration (`nacc-github` stub); security hardening (`nacc-policy`, `nacc-secrets` stubs — policy engine, redaction, protected paths); packaging/release docs (installer + updater pipeline exists in CI; user guide, threat model, clean-machine smoke tests missing).

**Acceptance evidence (master plan §20/§27):** the 20-step end-to-end demo on a safe repo, cancellation-kills-process-tree demonstration, rate-limit fallback visibility, secret redaction, protected-path blocking, dirty-worktree quarantine, clean-machine install — NONE demonstrated yet. Do not claim completion without them.

---

## 8. What still requires CI or the user (do NOT fake it)

- Everything pushed goes through `.github/workflows/ci.yml` (fmt → clippy -D warnings → workspace tests → export-bindings → npm build/test → signed NSIS installer, Windows-only). Local GNU checks do NOT substitute for CI (MSVC behavior is unverified locally).
- **Do not push, do not dispatch CI, do not publish releases** unless the user explicitly authorizes it. Repo convention: local commits + honest documentation; the user pushes.
- Live provider detection/auth against real installed CLIs has never been exercised from the GUI (the probes are real code paths, but no live run was demonstrated). A `tauri dev` smoke launch is a manual step on this machine.
- The Windows SDK libs needed for MSVC linking are missing; installing them (or accepting CI as the MSVC gate) is the user's call.

---

## 9. Key file map

- Specs: `native-agent-control-center-tauri2-rust-master-plan.md`, `native-agent-control-center-tauri2-rust-build-prompt.md`
- Status: `docs/architecture/status.md` (detailed per-phase state + verification evidence), this file, `docs/architecture/overview.md`, `docs/adr/0001-foundation-selection.md`, `docs/adr/0002-provider-transport.md`, `docs/audits/foundation-audit.md`, `docs/provider-adapters/*.md` (per-CLI verified contracts — read before touching an adapter)
- Crates: `crates/nacc-{domain,storage,events,orchestrator,provider-core,provider-claude,provider-codex,provider-antigravity,provider-copilot,provider-opencode,process,runtime,worktree,git,github,policy,quality,review,secrets,observability,updater}`
- App: `src-tauri/src/{lib.rs,main.rs,diagnostics.rs,providers.rs,role_profiles.rs,routing.rs,executor.rs,workflows.rs,build.rs}`, `src-tauri/tauri.conf.json`
- Frontend: `src/{App.tsx,App.test.tsx,RoleMatrix.tsx,RoleMatrix.test.tsx,Providers.tsx,Providers.test.tsx,main.tsx,App.css,RoleMatrix.css}`, `src/bindings.ts` (generated, gitignored)
- CI: `.github/workflows/ci.yml` (read its header comments — they encode hard-won lessons about workspace target paths and `if-no-files-found: error`)

---

## 10. Honesty requirements for the final report

Follow build prompt §22: report the foundation decision, architecture, adapters + tested CLI versions, GUI pages completed, security controls, tests/builds with exact results, end-to-end evidence, remaining limitations tied to provider contracts, files changed + ADRs, artifact locations, and actions deliberately not performed. **Never** claim "100% complete", "bug-free", or provider support that was not demonstrated live. Phase completion requires CI evidence, not local green checks alone.

---

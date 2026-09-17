# NACC implementation status and agent handoff

## Provider detection checkpoint — 2026-09-17 (updated)

Role Matrix milestone is committed locally as `2403554` (no push). This slice adds the real Claude/Codex registry in `AppState`, typed `detect_provider`/`list_provider_installations` IPC, persisted native-Windows installation observations, and a Providers panel. Detection is explicit, version-command-only, bounded to 15 seconds, and runs only on button click. Adapters return command names, not resolved executable paths; an unsuccessful probe may mean a missing or broken launcher. Authentication, readiness, and model availability are not inferred. Antigravity/Copilot/OpenCode and WSL2/Docker detection remain unwired.

Verified: GNU build and export passed; both generated bindings present; adapter suites 25/19 and storage 48 tests passed; frontend build plus 15 Vitest tests passed; app clippy clean with `-D warnings`; **plain `cargo test -p nacc-app` passes 6/6** after the build-script fix below. No live provider detection against installed CLIs and no visual desktop end-to-end run have been demonstrated for this slice.

**GNU test-loader root cause found and FIXED (build.rs):** ordinary `cargo test -p nacc-app` used to abort before test startup with `0xc0000139`. `dumpbin /dependents` ruled out ICU/VC++ redistributables. The test binary imports `TaskDialogIndirect` from `comctl32.dll` but had no `.rsrc` section — the app binary embeds Tauri's Common Controls v6 manifest via `cargo:rustc-link-arg-bins`, which only reaches binary targets. Fix in `src-tauri/build.rs`: for Windows GNU targets only, emit `cargo:rustc-link-arg` pointing at Tauri's already-generated `OUT_DIR/libresource.a`, so the manifest links into the test harness as well (duplicate static inclusion into bins is harmless; export re-verified). Verified: `cargo +1.96.0-x86_64-pc-windows-gnu test -p nacc-app` passes; `cargo ... run -p nacc-app -- --export-bindings` still writes bindings and runs. The `cargo:rustc-link-arg-tests` instruction is NOT valid in this Cargo version (whole build fails with "invalid instruction") — do not use it. MSVC-side test behavior still needs its own verification on a machine where MSVC links.


## Local continuation checkpoint — 2026-09-17

This checkpoint supersedes older current-state statements below; it does not mark Phase 6 complete.
HEAD remains `bdd682a`; these continuation changes are uncommitted.

- Completed the interrupted `RoleProfileView` conversion in create/update IPC responses; list/create/update consistently expose string timestamps.
- Replaced the local, gitignored bindings stub by running the real GNU-toolchain `nacc-app --export-bindings` executable successfully. Do not recreate handwritten bindings.
- Added and mounted a Role Matrix CRUD screen using the generated Tauri commands: create/edit, custom roles, independent provider/model assignment, enable/disable, confirmed deletion, loading/error/retry states, and duplicate-submit/stale-response protection.
- Capability discovery is NOT connected. Model IDs and permissions are requested configuration, not validated or applied settings. Thinking/reasoning selectors are disabled, while existing stored values are preserved on edit.
- Verified locally: GNU `cargo check -p nacc-app`; GNU `cargo test -p nacc-storage --lib` (48 passed); frontend `npm run build`; `npm test` (11 passed, including 7 Role Matrix tests). Frontend tests mock the IPC boundary; storage tests exercise SQLite. These are not a real desktop UI-to-SQLite end-to-end demonstration.
- A proposed Tauri MockRuntime IPC test compiled but its test executable failed at startup with `0xc0000139` (`STATUS_ENTRYPOINT_NOT_FOUND`) locally. Its test-only changes were reverted; do not report it as passing. MSVC validation still requires a correctly configured Windows build environment / CI.
- No CI dispatch, push, release, production operation, or provider authentication was performed.

Next: wire capability/model discovery and native onboarding, extend role profiles for account/runtime/fallback configuration, then connect the durable workflow engine and verify a real safe-repository run. Phases 8–12, remaining Phase 3/5 acceptance gaps, and clean-machine/installer/security acceptance evidence remain outstanding. The detailed roadmap below remains applicable except for outdated claims that the frontend contains only diagnostics or IPC contains only one command.


**Purpose of this document.** A self-contained handoff so a new implementation
agent can continue NACC with no prior session context. It records *what is
actually built*, *what is not*, *the exact uncommitted state of the working
tree*, and the ordered work that remains — with the verification method for
each item.

**Audience.** An AI implementation agent (or human engineer) picking this
repository up cold.

**Status as of:** commit `db2588d` ("Document Phase 7 and the real
local-build environment"), branch `main`. The Phase 7 engine and storage work that this
document once recorded as an unprotected uncommitted vault is now
**committed and CI-verified** (§6); Tasks A and B of the work plan (§9) are
done. Next up: Task C (Phase 6 GUI) or Task D (Phase 7 wiring into the
application).

---

## 1. Mission and binding specification

NACC (Native Agent Control Center) is a Windows-first, local-first desktop GUI
that orchestrates multiple native coding-agent CLIs (Claude Code, OpenAI Codex,
Google Antigravity, GitHub Copilot, OpenCode/external gateways) plus GitHub
Actions for deterministic CI/CD. It is **not** an LLM. It is a role router,
workflow engine, process supervisor, permission broker, worktree manager,
review console, and CI/CD control center.

Two documents are binding. Read both fully before changing anything:

| Document | Role |
|---|---|
| `native-agent-control-center-tauri2-rust-master-plan.md` (2090 lines) | Governing product and architecture specification. |
| `native-agent-control-center-tauri2-rust-build-prompt.md` (688 lines) | Direct execution prompt; phase sequence and verification gates. |

**Conflict rule (from master plan §32):** where a current provider contract or
security limitation conflicts with a requirement, document the evidence in an
ADR, preserve the *intent* of the plan, and choose the safest maintainable
implementation. Never silently weaken a requirement.

### Non-negotiable constraints (build prompt §2)

1. Tauri 2 (latest stable, pinned). **Never Electron.**
2. Privileged backend and orchestration engine are **Rust**.
3. React + TypeScript + Vite frontend inside the webview.
4. **No Node.js/Python/Go/.NET application server at runtime.** Node is
   frontend tooling only.
5. All process execution, PTY supervision, workflow transitions, persistence,
   Git mutations, worktree management, policy enforcement, secrets access,
   audit, and GitHub automation are Rust-owned.
6. **The webview must never receive a general shell or filesystem capability.**
7. SQLite with Rust-managed embedded migrations for durable state.
8. Windows-native process-tree containment (Job Objects).
9. Separate Git worktrees for write-enabled parallel workers.
10. GitHub Actions remains the deterministic CI/CD and deployment engine; agents
    diagnose and repair CI but never replace it.
11. Production deploy, destructive DB operations, secret changes, repo-visibility
    changes, branch-protection changes, and force-pushes to protected branches
    stay explicitly approval-gated.
12. Provider-native subscription credentials remain provider-owned — never
    copy or extract OAuth tokens. NACC-owned API keys go to Windows Credential
    Manager or a documented encrypted fallback.
13. **No hard-coded model marketing names** as architectural constants. Discover
    or validate models and capabilities dynamically.
14. **Never pretend a provider setting was applied.** Unsupported controls are
    disabled or surfaced as provider-managed.
15. **Never store or expose hidden chain-of-thought.** Persist visible plans,
    summaries, tool events, commands, outputs, and validated handoff artifacts
    only.

---

## 2. Environment reality — read before trying to build

- **Local Rust linking is impossible on this machine under the pinned
  MSVC target** (MSVC Build Tools' compiler *is* installed, but the
  Windows SDK import libraries — `kernel32.lib` et al. — are not, so
  every link fails, including build scripts, which means even
  `cargo check` and `cargo clippy` fail). **But tests CAN run locally**
  via a GNU-host toolchain; this was proven in the Task A session:
  ```
  rustup toolchain install 1.96.0-x86_64-pc-windows-gnu --profile minimal
  cargo +1.96.0-x86_64-pc-windows-gnu test -p nacc-domain -p nacc-storage \
        -p nacc-orchestrator --all-features
  ```
  (the scoop-installed `gcc` on PATH is the linker; the `+` override beats
  `rust-toolchain.toml`). This works for the library crates; `nacc-app`
  (Tauri/WebView2) is untested locally and remains CI territory. The same
  toolchain runs **the full workspace's clippy step locally** — clippy only
  needs to link build scripts and proc-macros, which the GNU toolchain
  handles:
  ```
  rustup component add --toolchain 1.96.0-x86_64-pc-windows-gnu clippy
  cargo +1.96.0-x86_64-pc-windows-gnu clippy --workspace --all-targets \
        --all-features -- -D warnings
  ```
  Run that before every push: two CI failures this session were clippy
  lints that plain `cargo test` locally cannot surface.
- **CI remains the authoritative gate anyway.** Local runs are a fast
  feedback loop, never evidence. Repository: `jerryboganda/NACC`, default
  branch `main`, Windows-only CI by standing directive. The project's own
  history contains a case
  where a green step silently uploaded nothing (see `ci.yml`'s header
  comment) — always confirm artifacts with:
  ```
  gh api repos/jerryboganda/NACC/actions/runs/<run-id>/artifacts
  ```
  and check for a genuine, non-expired, SHA256-digested installer artifact.
- Two shell traps that cost real time: piping cargo through `tail`
  reports *tail's* exit code, not cargo's (`cargo check | tail` "passing"
  while compilation failed); and Git Bash's `/usr/bin/link` (coreutils)
  shadows MSVC's `link.exe` when one is reachable, producing
  `link: extra operand` noise instead of a clear SDK error.
- `src/bindings.ts` is **generated and gitignored**. It is produced by
  `cargo run -p nacc-app -- --export-bindings`, which calls the same
  `specta_builder()` the app uses. **Never hand-edit or commit it.**

### CI pipeline (`.github/workflows/ci.yml`)

Runs on every push to `main`, `windows-latest`, in this order:

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
3. `cargo test --workspace --all-features`
4. `cargo run -p nacc-app -- --export-bindings`
5. confirm `src/bindings.ts` exists
6. `npm ci`
7. `npm run build` (tsc + vite, type-checks against the real generated bindings)
8. `npm test` (Vitest)
9. `npx tauri build` (signed NSIS installer + updater artifacts)
10. upload NSIS installer + `.sig` (`if-no-files-found: error`)

**Step order is load-bearing:** Rust steps must run before the frontend build
because the latter imports the freshly generated `bindings.ts`.

Also present: `.github/workflows/foundation-audit.yml` — Phase 0 evidence only.
It builds **upstream AgentPanel** at a pinned SHA, not this repository. Do not
extend it as ongoing CI.

### Working method that has been used successfully

Small, reviewable commits by phase → push to `main` → read the actual CI result
→ fix real errors (each early phase failed CI on a genuine, distinct bug that
was then fixed in sequence, not guessed around) → verify artifacts. Do not batch
multiple phases into one push; the CI signal becomes unreadable.

---

## 3. Pinned toolchain (verified, not assumed)

| | Version | Note |
|---|---|---|
| Rust | 1.96.0 (`rust-toolchain.toml`) | Real floors: Tauri 2.12 MSRV 1.90, specta rc.25 needs `std::fmt::from_fn` (stable 1.93.0). 1.90 was tried and genuinely failed CI with `E0658` inside specta. |
| `tauri` | 2.11.5 | |
| `tauri-build` | 2.6.3 | default `config-json` feature required (JSON config, not JSON5/TOML) |
| `tauri-plugin-updater` | 2.10.1 | |
| `specta` / `tauri-specta` / `specta-typescript` | `=2.0.0-rc.25` / `2.0.0-rc.25` / `0.0.12` | `specta` is **exact-pinned** (`=`) because the crate doc warns to during the beta period. |
| React / Vite / TypeScript / Vitest | 19.2.8 / 8.2.2 / 7.0.2 / 4.1.11 | |

`workspace.package.rust-version = "1.93"`, `edition = "2021"`, `license = "MIT"`,
`publish = false`.

**Dependency policy:** dependencies used by a later phase are added **only when
that phase starts**, after their current contract has been checked. Never
predeclare speculatively. This is stated in the root `Cargo.toml` and has been
followed consistently.

---

## 4. Repository structure

```
Cargo.toml                 # workspace root, resolver = "2", 21 crates + src-tauri
rust-toolchain.toml        # pinned 1.96.0
src-tauri/                 # package name is `nacc-app`; Tauri 2 composition root
crates/                    # 21 library crates (see §5)
src/                       # React + TypeScript + Vite frontend
docs/                      # adr/, architecture/, audits/, provider-adapters/
.github/workflows/         # ci.yml, foundation-audit.yml
```

**Dependency-direction rule:** every crate depends only on `nacc-domain` (or, for
provider crates, on `nacc-provider-core`, which itself depends only on
`nacc-domain`) plus whatever external crates it genuinely uses. `src-tauri` is
the only crate that depends on all of them — it is the composition root.
Provider crates must never depend on GUI components.

### Still-missing top-level paths from the master plan (§6 / build prompt §13)

| Path | Why it is needed |
|---|---|
| `schemas/` | 5 versioned JSON Schemas: task contract, agent handoff, review result, quality-gate result, CI failure diagnosis. |
| `presets/` | Source-controlled representations of workflow/role presets. |
| `tests/` | `fixtures/`, `adapter-contracts/`, `integration/`, `e2e/`. **No integration test directory exists anywhere yet.** |
| `docs/security/`, `docs/operations/`, `docs/user-guide/` | Required documentation deliverables (build prompt §19). |
| `NOTICE` | MIT attribution obligation for the 5 AgentPanel techniques (see `docs/upstream-delta.md`). |
| `migrations/` (SQL files) | Not needed — migrations are embedded Rust (`nacc-storage/src/migrations.rs`), which is the deliberate design. |

---

## 5. Crate-by-crate state

Legend: **REAL** = production logic · **STUB** = boundary + typed error only,
methods return "not implemented yet".

| Crate | Phase | State | What is actually there |
|---|---|---|---|
| `nacc-domain` | 1–2 | **REAL** | Strong IDs via `define_id!` (`ProjectId`, `WorkflowRunId`, `RoleProfileId`, `ProviderAccountId`, `NodeRunId`, `AttemptId`, `EventId`, `AuditEventId`, `ApprovalId`, `CapabilitySnapshotId`, `WorktreeLeaseId`, …), `ProviderId` (closed enum), `ModelId`, `ReasoningLevel`, `ThinkingMode`, `PermissionProfile` (incl. `rank`/`narrower_of`), `RoleKind` (18 roles + `Custom`), `RoleProfile`, `WorktreeState`, `WorktreeLease`, `RunState`, `NodeState`, `AttemptTrigger`, `NodeFallback`, `WorkflowNode`, `WorkflowTemplate`, `ApprovalDecision`. |
| `nacc-events` | 2 | **REAL** | `Event` — the closed 20-variant normalized vocabulary from master plan §8.2 (deliberately no `Other(String)` escape hatch). `AuditRecord` — the audit-trail shape §22 requires. Pure domain types, no SQLite. |
| `nacc-storage` | 2, 7 | **REAL** | `Database::{open, open_in_memory, backup_to, restore_from}` (WAL, `foreign_keys` pragma, `VACUUM INTO` backup). Migrations V1–V5. Repos: settings, role_profiles, events, audit, worktree_leases, providers/capabilities, workflow (runs/node runs/attempts/approvals/checkpoints). Every DB-touching method is `async` and runs the synchronous rusqlite call inside `tokio::task::spawn_blocking`. |
| `nacc-events` / `nacc-observability` | 1 | **REAL** | See rows above; `init_tracing(log_dir, dev_mode)` (daily-rotating JSON file + console, one `NACC_LOG` filter) and `workflow_run_span`. |
| `nacc-provider-core` | 4 | **REAL** | `AgentProvider` async trait (dyn-compatible), `CapabilitySnapshot` (full §8.3 set), `InstallationProbe`/`AuthProbe`/`ModelDescriptor`, `ProviderHealth::from_probes` (not-installed outranks all; ineligible ≠ unauthenticated), `ProviderRegistry`, 21-variant `ProviderEvent` + `EventSink`, `CommandRunner` / `ProcessCommandRunner` (over `nacc-process`) / `FixtureCommandRunner`, `SessionSupervisor`, `launch_streaming_session`, and a real 9-check **contract suite** (`contract.rs`) proven able to fail. |
| `nacc-provider-claude` | 5 | **REAL (fixture-only execution)** | Permission-mode mapping (refuses to reach `bypassPermissions` except the temporary profile), reasoning mapping with visible clamp flags, launch/resume argv, `stream-json` parser, tools-keys-only summaries (no secret leakage), exact usage from `total_cost_usd` + tokens, install probe via `--version`, auth probe by credential-store **existence only**, full trait impl, contract-suite test. |
| `nacc-provider-codex` | 5 | **REAL (fixture-only execution)** | Sandbox/approval-policy mapping, reasoning → `model_reasoning_effort`, launch/resume argv, JSONL parser, usage reported as `Unknown` rather than invented, install probe + `~/.codex/auth.json` existence, full trait impl, contract-suite test. Reports `cancellation_documented: false`. |
| `nacc-provider-antigravity` | 8 | **STUB** | All methods return `ProviderError::Other("… not implemented yet — Phase 8 scope …")`. Only `id`/`display_name` real. Blocked on finding a real headless interface (Phase 0 found only the IDE GUI, no `agy` CLI). |
| `nacc-provider-copilot` | 5/10 | **STUB** | Same pattern. Doc comment records a live-verified CLI contract (`--output-format json`, `--acp`, effort scale) but none of it is implemented. |
| `nacc-provider-opencode` | 8 | **STUB** | Same pattern. Blocked on a working local binary (the audited machine's OpenCode CLI is broken — missing `opencode-cli.exe`). |
| `nacc-process` | 3 | **REAL** | Windows Job Objects (`CreateJobObjectW`, `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, `AssignProcessToJobObject`, `TerminateJobObject`), `ProcessSpec` validation (refuses empty program, NUL bytes, invalid env names, missing cwd), env allowlist, `CREATE_NO_WINDOW`, line-framed reader tasks, graceful (close stdin + bounded wait) → forced (job terminate), `wait` with output linger, `Drop` terminates a running tree. Honest `UnsupportedPlatform` off-Windows. |
| `nacc-git` | 3 | **REAL** | Typed git CLI wrapper with argument arrays (never interpolated shell): version, init, open-validity, resolve-commit, configure-identity (`--local` only), stage-all, commit (fails loudly if nothing staged), branch/head queries, worktree list/add/remove (bounded retry), dirty / ahead / unpushed checks, prune, porcelain parser, `sanitize_branch_segment`. Tests run against real temp repos. |
| `nacc-worktree` | 3 | **REAL** | Lease lifecycle: deterministic collision-safe naming (`nacc/<label>-<8hex>`), allocate (resolves base to a fixed SHA, persists lease before returning), inspect (MissingPath/NotRegistered/BranchChanged/HeadMoved/DirtyWorkingTree/UnintegratedCommits/DetachedHead), release (`RemoveIfSafe` only for clean + fully-integrated; otherwise quarantine), quarantine (renames dir, refuses overwrite, writes `NACC-QUARANTINE.json`, prunes stale registration best-effort), reconcile (live owner left alone; absent → AlreadyAbsent; must-preserve → quarantine; clean → release; unmanaged worktrees reported but never removed). |
| `nacc-runtime` | 3 | **REAL (detection)** | Probes for Git, `gh`, WSL2, Docker, VS Code, Antigravity, treating "not installed" as a normal typed result. Handles `wsl.exe` UTF-16LE output (verified, not assumed). `wrap_for_target`. **Deferred:** real WSL2/Docker mount + working-directory mapping design. |
| `nacc-orchestrator` | 7 | **REAL** | Pure `scheduler` (validate/cycle/readiness/outcome), pure `governor` (global/project/provider caps, RAII permits), `clock` (system + virtual), `engine` (state machine, checkpoints, approvals, fallback, permission narrowing), `recovery` (startup reconcile; never auto-resumes), `template` (4 built-in DAGs). See §6. |
| `nacc-policy` | 11 | **STUB** | Placeholder `PolicyError::Other` + display test. |
| `nacc-quality` | 9 | **STUB** | Placeholder `QualityError::Other` + display test. |
| `nacc-review` | 9 | **STUB** | Placeholder `ReviewError::Other` + display test. |
| `nacc-secrets` | 11 | **STUB** | Placeholder `SecretsError::Other` + display test. |
| `nacc-github` | 10 | **STUB** | Placeholder `GithubError::Other` + display test. |
| `nacc-updater` | 12 | **STUB** | Placeholder `UpdaterError::Other` + display test. Dev keypair lives in `src-tauri/tauri.conf.json`. |
| `src-tauri` (`nacc-app`) | 1–2 | **REAL (thin)** | `specta_builder()`, `bindings_output_path()`, `run()`, `AppState { diagnostics_run_id, storage, _tracing_guard }`, and exactly **one** command: `diagnostics::get_app_diagnostics`. |

---

## 6. Phase 7 — committed (was the uncommitted vault)

The substantial Phase 7 (durable workflow engine) implementation this
document once recorded as unprotected uncommitted work is now committed on
`main` in four commits, protected and CI-verified:

| Commit | Contents |
|---|---|
| `42ecd27` | `nacc-domain`: workflow state-machine types (RunState, NodeState, AttemptTrigger, NodeFallback, WorkflowNode, WorkflowTemplate, ApprovalDecision + PartialEq — its absence broke a domain test the first time the code was ever compiled-and-run), ApprovalId, PermissionProfile::rank/narrower_of. |
| `b5a99ef` | `nacc-storage`: V5 migration (workflow_runs, node_runs, node_attempts, approvals, run_checkpoints + 5 indexes), the workflow repository, V3→V4 and V4→V5 upgrade tests, and a 9-test module for the new repository. |
| `2de42ee` | `nacc-orchestrator`: scheduler, governor, clock, engine (1332 lines), recovery, templates, 50 tests, plus the `nacc-storage` dependency. |
| `061b003` | Clippy fix (let_and_return) the first CI run caught — the one check local verification could not reproduce before the GNU-toolchain recipe below existed, since clippy must link build scripts. |
| `86515ff` | Two more clippy lints in test code (`contains()`, `vec!`→array) — the last push made the full workspace clippy step runnable locally, so this class of failure stays local from now on. |
| `db2588d` | Documentation (this file, overview.md) and `.gitignore` hygiene. |

**Verified by CI run `35164045996`** (HEAD `db2588d`): green end-to-end with
a genuine installer artifact (`nacc-windows-installer-db2588d…`,
4,229,391 bytes, confirmed via the artifacts API). Locally first: 114 tests
across
the three crates on the GNU-host toolchain, §2). Two latent bugs were found
and fixed the first time this code actually ran, both in commit `2de42ee`:
the recovery-requeue trigger was misclassified as `Retry` (a Pending node
with past attempts and no cooldown entry is a recovery requeue, not an
engine-scheduled retry), and the overlap test's barrier wrongly gated the
dependent third node.

### What the code does

**`nacc-storage/src/migrations.rs`** — adds `V5_WORKFLOW_STATE`: tables
`workflow_runs`, `node_runs`, `node_attempts`, `approvals`, `run_checkpoints`
plus 5 indexes. Version assertions updated 4 → 5.

**`nacc-storage/src/workflow.rs`** (673 lines) — 17 async methods:
- runs: `insert_workflow_run`, `update_workflow_run`, `get_workflow_run`,
  `list_workflow_runs_in_states`, `list_workflow_runs_for_project`,
  `delete_workflow_run` (manual cascade)
- node runs: `insert_node_run`, `update_node_run`, `list_node_runs`
- attempts: `insert_node_attempt`, `finish_node_attempt`, `list_node_attempts`
- approvals: `insert_approval`, `decide_approval` (pending-only guard),
  `list_approvals`
- checkpoints: `append_checkpoint` (per-run sequence), `list_checkpoints`

**`nacc-orchestrator`** — 6 new modules:
- `scheduler.rs` — pure DAG functions: `validate` (empty key, duplicates,
  self-dep, unknown dep, cycle, no entry point), `cycle_members`, `Readiness`,
  `runnable`, `newly_blocked`, `outcome`, `failures`, `fully_instantiated`.
- `governor.rs` — pure concurrency accounting: global/project/provider caps,
  `Capacity` reason enum, RAII `Permit`, `try_acquire`/`release`. Deliberately
  no queue.
- `clock.rs` — `Clock` trait, `SystemClock`, `VirtualClock::advance` so retry
  backoff is testable without sleeping.
- `engine.rs` (1332 lines) — the state machine. `RetryPolicy` (exponential
  backoff), `EngineConfig`, injectable `NodeExecutor` and `RoleRouting`
  boundaries, `StaticRouting`, `RunSnapshot`, `Persistence` (every transition is
  a checkpoint), lifecycle `start_run`/`start_and_run`/`run`/`resume`/`pause`/
  `cancel`/`decide_approval`/`approve_and_resume`, `drive` loop (skip-blocked →
  runnable → approval gate → cooldown → dispatch), `finish`, `await_approval`,
  `requeue_unfinished`, `mark_interrupted`, `dispatch` (JoinSet + governor
  permits), `run_attempt`.
- `recovery.rs` — startup `reconcile`: finds Running/AwaitingApproval runs,
  closes orphaned attempts, marks `Interrupted`, is idempotent, and **never
  auto-resumes**.
- `template.rs` — 4 built-in DAGs as real executable structures.
- `engine/tests.rs` (1045 lines) — 22 tests using a real in-memory DB + virtual
  clock + scripted fake executor: happy path ordering, parallelism (barrier),
  governor peak enforcement, failure/skip propagation, retry with real backoff,
  attempt exhaustion, missing CLI, executor panic, approval gate (approve/
  reject/immutability/re-run cannot bypass), routing fallback with recorded
  reason, unassigned role failure, permission narrowing, pause/cancel/resume,
  cyclic template refused before any write, crash recovery, approval survives
  restart.

Test totals as committed: orchestrator **50 tests** (engine 22, governor 8,
template 8, scheduler 7, clock 3, lib 2), storage **43** (32 pre-existing +
9 new workflow-repository tests + 2 new migration-upgrade tests V3→V4 and
V4→V5), domain **21** = **114 total**, all green locally and in CI.

### Known gaps (do not forget these)

1. **No Tauri wiring at all.** `src-tauri/src/lib.rs` does not reference the
   orchestrator; `AppState` has no engine handle; only `get_app_diagnostics` is
   exposed. A user cannot start a run.
2. **No production `NodeExecutor` and no production `RoleRouting`** exist
   anywhere outside test fakes. Nothing launches a provider CLI.
3. **Pause/cancel are cooperative only.** They set DB state but cannot abort
   in-flight agent tasks.
4. **No per-node timeout** and **no attempt leases/heartbeats** (both required
   by master plan §14.1/§14.2).
5. **Templates cover 4 of 6 §18 presets.** Missing: *Frontend Visual Hardening*
   (18.4), *Backend Security Change* (18.5). The three present
   (Enterprise Feature, Fast Bug Fix, CI/CD Repair) are simplified vs. §18.
6. **`RunState` simplifies §14.1's 16 states to 8** (`Pending`, `Running`,
   `Paused`, `AwaitingApproval`, `Interrupted`, `Succeeded`, `Failed`,
   `Cancelled`). `Draft`/`Preflight`/`Superseded`/`RequiresManualIntervention`
   are absent.
7. **`AttemptTrigger::Repair` is defined but never emitted** (bounded repair is
   Phase 9 scope).
8. **No foreign-key constraints in V5** despite the `foreign_keys` pragma being
   enabled; cascade is manual.
9. **No fallback chains attached** to any built-in template node (all
   `fallbacks: vec![]`).

---

## 7. Storage schema and data-group coverage

### Migrations

| Version | Adds |
|---|---|
| V1 | `app_settings`, `role_profiles`, `events`, `audit_events` |
| V2 | correlation indexes on `events` and `audit_events` (`workflow_run_id`, `node_run_id`) |
| V3 | `worktree_leases` (+ project/run indexes); columns include `state_json`, `owner_process_id`, `quarantine_reason` |
| V4 | `provider_installations` (PK `(provider_json, runtime_json)`), `capability_snapshots` (append-only) |
| V5 | `workflow_runs`, `node_runs`, `node_attempts`, `approvals`, `run_checkpoints` (+ 5 indexes) |

All enum-typed columns are stored as that type's own `serde_json` encoding, not a
second hand-written `Display`/`FromStr` mapping — one source of truth for a
type's wire **and** storage representation.

### §4.4 data groups

| Data group | Schema + repository? |
|---|---|
| application settings | ✅ V1 |
| role profiles | ✅ V1 |
| event stream | ✅ V1 |
| audit records | ✅ V1 |
| worktree allocations | ✅ V3 |
| provider installations | ✅ V4 |
| discovered models / capability snapshots | ✅ V4 (models embedded in `snapshot_json`) |
| workflow runs, node attempts | ✅ V5 |
| approvals | ✅ V5 |
| **account references and health state** | ⬜ (only `health` inside snapshot JSON) |
| **workflow templates and versions** | ⬜ (templates are built-in code; `workflow_runs.template_name` is a plain string) |
| **task contracts and handoffs** | ⬜ |
| **quality-gate results** | ⬜ |
| **review findings** | ⬜ |
| **CI/CD records** | ⬜ |
| **policy decisions** | ⬜ |
| **usage estimates** | ⬜ |
| **updater and diagnostic state** | ⬜ |
| **artifact references** | ⬜ (required by build prompt §13, master plan §17.10) |

This deferral is deliberate scope discipline (documented in `nacc-storage`'s
crate doc): each group gets its migration when its owning phase has real logic
to back it, not schema'd speculatively. Do not "fix" it by pre-creating empty
tables.

---

## 8. Phase roadmap — done vs. remaining

### ✅ Phase 0 — Foundation audit and decision gate
`docs/audits/foundation-audit.md`, `docs/adr/0001-foundation-selection.md`
(**greenfield, NOT an AgentPanel fork** — criterion #9 "untestable monolith"
failed), `docs/adr/0002-provider-transport.md` (bespoke per-provider adapters;
ACP evaluated per-provider, not adopted blanket), `docs/upstream-delta.md`
(5 techniques reimplemented; license obligation recorded),
`docs/provider-adapters/*.md` (live-probed contracts), `foundation-audit.yml`.

### ✅ Phase 1 — Tauri/Rust modular foundation
21-crate workspace, pinned toolchain, typed IPC end-to-end, tracing, strict CSP
(not `null`), minimal capabilities (`core:default` + `updater:default` only),
NACC's own updater signing key (private key in GitHub Actions secrets:
`TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; public key in
`tauri.conf.json`), signed NSIS installer produced in CI.

### ✅ Phase 2 — Durable domain and storage
See §7. Verified by CI run `33330656904` (all 21 steps) with a real
4,224,969-byte installer artifact.

### ✅ Phase 3 — Process, runtime, and worktree core
`nacc-process` (Job Objects), `nacc-git`, `nacc-worktree`, `nacc-runtime`.
Note: PTY/ConPTY is **deliberately deferred** — no implemented adapter needs a
pseudo-console for the non-interactive modes they launch in, and
`CapabilitySnapshot` reports `interactive_pty` as a real per-provider fact so
the absence is visible, not hidden. `portable-pty` is the right library when a
consumer exists.

### ✅ Phase 4 — Provider registry and adapter framework
Contract layer + 9-check contract suite + registry + health + normalization.

### ✅ Phase 5 — Claude and Codex MVP adapters (with honest caveats)
Adapters are real, but:
- Only `--version` fixtures are **live-captured**. Stream/JSONL fixtures are
  explicitly *documented-shape*, `captured_from_version: null` (see each
  crate's `fixtures/README.md`).
- **No adapter is wired outside its own crate.** `ProcessCommandRunner` is never
  constructed in production; `ProviderRegistry` is never populated. Live CLI
  launch is therefore **not verified**.

### ✅ Phase 7 — Durable DAG orchestration (core; wiring remains)
Committed as `42ecd27`/`b5a99ef`/`2de42ee` (+ clippy fixes `061b003`,
`86515ff`), CI-verified by run `35164045996`. What remains for the engine
to be *usable* is Task D in §9
(Tauri wiring, a production `NodeExecutor`/`RoleRouting`, provider-registry
population, per-node timeouts and attempt leases, template versioning plus
the two missing §18 presets and fallback chains) — see §6's gap list.

### ⬜ Phase 6 — Setup Wizard and Role Matrix GUI
- All 17 GUI pages (only a diagnostics screen exists).
- Setup wizard: detect CLIs/Git/gh/Rust/WSL2/Docker/VS Code, show exact paths
  and versions, native login flows, auth verification, model/capability
  discovery, harmless read-only smoke prompt, secrets-storage config.
- Role Matrix spreadsheet editor: 18 roles × (enabled, provider, account,
  model/alias, reasoning, thinking, permission, runtime, working-dir strategy,
  max turns, time limit, context budget, concurrency, allowed/denied tools,
  network policy, MCP/plugin profile, structured-output requirement, fallback
  chain, retry policy, budget, approval policy, reviewer-separation rule),
  plus compare mode, presets, bulk changes, fallback editor, effective-setting
  preview, and blocking of invalid combinations.
- Model Catalog, Workflow Designer (React Flow), Live Run Center,
  Worktree Manager, Review Center, Quality Gates, CI/CD Center, Usage/Quotas,
  Security/Policies, Audit Log, Diagnostics/Updates.
- **Frontend stack still entirely absent** from `package.json`: TanStack Query,
  Zustand (or equivalent), React Flow, xterm.js, Monaco, component system,
  Playwright. Do not add these speculatively — add each when the page that needs
  it starts.
- `RoleProfile` already has tested storage; only the GUI is missing.

###  Phase 8 — Antigravity and OpenCode adapters
Both are stubs. Antigravity needs a real headless interface (or WSL2 fallback)
plus schema-validated JSON handoff enforcement. OpenCode needs gateway profiles
(TokenRouter, B.AI, DeepSeek/GLM/Qwen families), model discovery,
reasoning/thinking parameter mapping, timeouts, concurrency, user-entered
pricing, and exact provider-returned model IDs (never assumed names).

### ⬜ Phase 9 — Review and quality system
`nacc-quality` and `nacc-review` are stubs. Need deterministic gate execution
with real evidence (command, cwd, env policy, exit code, duration, redacted
output, parsed results, artifacts), flakiness evidence that never erases the
initial failure, diff viewer, line-level findings with severity/evidence/
disposition/repair link, cross-provider reviewer enforcement, and bounded repair
(≤2 cycles per failure signature).

### ⬜ Phase 10 — GitHub and Copilot CI/CD
`nacc-github` and `nacc-provider-copilot` are stubs. Need repo/branch/PR/check/
workflow-run/job/step/log/annotation/artifact/environment integration, stable
Copilot programmatic mode, version-gated ACP, the 12-class CI failure taxonomy,
repair-run creation, rerun of only permitted workflows/jobs, staging status, and
production approval UI.

### ⬜ Phase 11 — Security and reliability hardening
`nacc-policy` and `nacc-secrets` are stubs. Need the policy engine, Windows
Credential Manager storage, redaction, protected path/command/network rules,
threat model, abuse-case tests, orphan reconciliation tests, and migration
upgrade tests from every prior schema version.

### ⬜ Phase 12 — Packaging, documentation, and release
`nacc-updater` is a stub (dev keypair only). Need production signing-key
rotation, a real release channel, clean-machine smoke test, user guide,
administrator/security guide, provider troubleshooting, backup/restore,
diagnostics bundle, release checklist, and the `NOTICE` attribution.

---

## 9. Ordered work plan for the next agent

Each task is independently committable and verifiable in CI.

### ✅ Task A — Protect and verify the Phase 7 work *(done, 2026-09-17)*
Done in this session, exceeding the original checklist:
1. Working tree matched §6 exactly.
2. Added the `workflow.rs` test module — **9 tests** covering insert→get→update
   (including `created_at_millis` immutability and unknown-id errors), node-run
   insertion-order listing, append-only attempt history, `decide_approval`
   double-decision refusal, per-run checkpoint monotonicity, full cascade
   delete, and the state/project filter query shapes.
3. Added migration upgrade tests for **V3→V4 and V4→V5**, both writing real
   rows under the old schema and asserting they survive.
4. Ran the tests locally for the first time (GNU-host toolchain, §2) — which
   surfaced **two latent bugs before CI did**: `ApprovalDecision` lacked
   `PartialEq` (a domain test could not compile), the recovery-requeue trigger
   was misclassified as `Retry`, and the overlap test's barrier wrongly gated
   the dependent third node. All fixed in the commits.
5. Committed in three logical commits (`42ecd27` domain, `b5a99ef` storage,
   `2de42ee` orchestrator) plus clippy fixes (`061b003`, `86515ff`), pushed,
   and read the real CI results: two runs failed clippy on three lints total
   (the one step local verification could not reproduce at the time), fixed,
   then **run `35164045996` green at HEAD `db2588d`** with a real
   4,229,391-byte installer artifact confirmed via the artifacts API.

### ✅ Task B — Documentation and hygiene *(done, 2026-09-17)*
1. `docs/architecture/overview.md` retitled to the Phase 1–7 range, stale
   "no workflow engine" claims replaced with the real Phase 3–7 state, and a
   Status pointer to this file added at the top.
2. `.freebuff/` and `output/` added to `.gitignore`.
3. This file updated throughout (§2 environment reality rewritten from the
   Task A discoveries, §5–§8 de-uncommitted, §9 task statuses, §12 facts).

### Task C — Phase 6: Setup Wizard + Role Matrix GUI
1. Introduce the frontend stack piecewise (TanStack Query for Rust-backed data;
   a small store for transient UI state; React Flow and Monaco only when their
   page is built).
2. Add typed Rust commands for the data each page needs (settings, role
   profiles, provider installations, capability snapshots, worktree leases) —
   thin wrappers over existing repositories, **never** raw SQL from the frontend.
3. Build the 17 pages from build prompt §8, starting with Setup Wizard and Role
   Matrix so the app becomes usable by a non-expert.
4. Enforce accessibility: keyboard access, visible focus, labels, contrast,
   loading/error/empty states, no hover-only controls.
5. **Done when:** CI is green, the Role Matrix round-trips real data through
   SQLite, and unsupported controls render disabled with an explanation.

### Task D — Phase 7 wiring: engine into the application
1. Add an orchestrator handle to `AppState`; expose typed commands for
   start/list/pause/cancel/resume/approve run, and for listing runs/nodes/
   attempts/checkpoints.
2. Implement a production `NodeExecutor` that launches a real provider session
   through `nacc-provider-core`'s `ProcessCommandRunner` and `nacc-runtime`, and
   a production `RoleRouting` over the persisted Role Matrix.
3. Populate `ProviderRegistry` with the Claude and Codex adapters.
4. Add per-node timeouts and attempt leases/heartbeats; make cancel able to
   abort in-flight work.
5. Persist workflow templates/versions so templates are versioned data, not only
   code; add the two missing §18 presets; attach fallback chains.
6. **Done when:** a real end-to-end run is observable in the GUI (explore →
   plan → implement → verify → review) with deterministic evidence, and a
   crash/restart reconciles safely.

### Tasks E–J — Phases 8 → 12
Follow the build prompt §17 sequence and the per-phase acceptance criteria in
master plan §27. Each phase must produce its own CI evidence and its own
documentation update. Do not mark a phase complete because screens or mocked
adapters exist (build prompt §17, final line).

---

## 10. Conventions and gotchas the next agent must follow

1. **Never claim something works without CI evidence.** Local Rust builds are
   not possible here. If a command is too costly locally, add a CI job and read
   the actual result.
2. **Verify artifacts, not checkmarks.** `if-no-files-found: error` is set for
   this reason; still confirm with `gh api`. A past green run uploaded nothing.
3. **Comments explain *why*, not *what*.** The existing code is unusually
   rationale-rich: doc comments record alternatives considered, the specific
   failure that drove a design, and what CI run proved it. Match that standard.
   Do **not** add comments that merely restate the code.
4. **Scope discipline.** Add a dependency, a migration, or a crate's real logic
   only when the owning phase starts. Never schema a data group speculatively.
5. **Storage patterns:** async methods running synchronous rusqlite inside
   `tokio::task::spawn_blocking`; one shared `Arc<Mutex<Connection>>`, not a
   pool; enum columns stored as that enum's `serde_json`; migrations are
   additive and versioned; use `VACUUM INTO` for backup, never a raw file copy of
   an open WAL database.
6. **Subprocess patterns:** executable + argument arrays, never interpolated
   shell strings; `CREATE_NO_WINDOW` on Windows; environment supplied by
   allowlist; process trees contained in a Job Object so cancellation leaves no
   orphans.
7. **Testing standard:** tests must exercise real behavior (real temp git repos,
   real in-memory SQLite, real multi-level process trees and an OS check that
   descendants are gone). Tests must be *proven able to fail* — the provider
   contract suite deliberately does this.
8. **Typed errors, deliberately split:** `thiserror` enums in libraries with
   specific variants for conditions the GUI must distinguish; `anyhow` only at
   application boundaries. A crate with no real logic yet has exactly one
   `Other(String)` variant, documented as a placeholder.
9. **Provider adapters:** preserve native auth, never extract tokens; map
   reasoning/permission/sandbox accurately; report unsupported controls as
   unsupported rather than silently downgrading; record requested *and* actual
   settings.
10. **Cargo.lock is committed** at the workspace root. Rust crates without a
    binary target do not need their own.
11. **`src/bindings.ts` is gitignored and generated.** Run
    `cargo run -p nacc-app -- --export-bindings` before any frontend type-check
    when working locally (where possible), or rely on CI's ordering.
12. **Do not push releases, alter repository visibility, deploy production, or
    change organization security settings** (build prompt §21).
13. **Do not reformat, reset, or delete unrelated files.** Preserve user work.

---

## 11. Acceptance criteria reminder (master plan §27)

The project is not complete until all 40 acceptance criteria are demonstrated
with reproducible evidence, including (non-exhaustive): Tauri 2 not Electron;
privileged behavior in Rust; no runtime Node server; installs on a clean Windows
machine; wizard detects exact CLI versions; native auth without exposing
credentials; at least Claude and Codex fully operational; models discovered not
assumed; every role configurable; unsupported controls disabled; requested vs.
actual settings audited; independent worktrees for writers; primary checkout
untouched; no two writers share a lease; process trees killed on cancel; runs
survive restart; handoffs schema-validated and cross-checked against Git and
command evidence; deterministic gates decide success; cross-provider review;
bounded repair; diff review and findings visible; GitHub PR/Actions visible;
failed CI creates a repair workflow; CI failures classified not just rerun;
production approval-gated; secrets in Credential Manager; redaction by default;
webview cannot invoke a shell; capabilities and CSP pass review; updater verifies
signatures; adapter tests detect incompatible CLI upgrades; migrations
upgrade-tested; pause/cancel/resume/quarantine/cleanup controls exist; VS Code
launch works; usage distinguishes exact/estimated/unavailable; no proprietary
cloud required; complete documentation; clean-machine smoke tests pass; and a
full end-to-end demonstration.

**Never claim "100% bug-free" or complete support where evidence is absent**
(build prompt §22).

---

## 12. Current exact facts (for quick verification)

- Current `HEAD`: `db2588d` "Document Phase 7 and the real local-build
  environment". Phase 7 commits, oldest first: `42ecd27`, `b5a99ef`,
  `2de42ee`, `061b003`, `86515ff`, `db2588d`.
- Last green CI run: `35164045996` (HEAD `db2588d`), with a real installer
  artifact (`nacc-windows-installer-db2588d…`, 4,229,391 bytes).
- Earlier verified runs: `35147797713` (Phase 5), `33332266695` (Phase 3
  part 1), `33330656904` (Phase 2, all 21 steps, real 4,224,969-byte
  installer artifact).
- Two historical failed runs, both understood: `35162519675` (Phase 7 part
  3, clippy let_and_return — fixed by `061b003`) and `35163001609` (two
  more clippy lints in test code — fixed by `86515ff`). One historical
  cancelled run: `35146915084` (a merge; reconcile it if it matters).
- Test counts as of `db2588d`: 268 tests across the workspace's library
  crates, all green locally on the GNU-host toolchain (domain 21, storage
  43, orchestrator 50, the rest unchanged from Phase 5); the `nacc-app`
  test target runs only in CI.
- Frontend surface today: `src/App.tsx`, `src/App.test.tsx`, `src/main.tsx`,
  `src/App.css`, `src/test/setup.ts`. No feature pages.
- `src-tauri` source files today: `diagnostics.rs`, `lib.rs`, `main.rs`. One
  command.
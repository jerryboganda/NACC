# NACC implementation status

**Updated:** 2026-09-19
**Current local-use readiness:** READY — single-user, local-only desktop scope

Public/distributed release readiness is intentionally **out of scope** for the
current product target. Signing, installer reputation, public update-channel
trust, clean-machine distribution testing, and protected release-candidate
evidence must not block personal local use.

This document is the current implementation ledger. Historical status checkpoints in Git history are not authoritative when they conflict with current source.

## Compute and operational policy

GitHub Actions is the preferred compute plane for heavy work: full Rust tests, clippy across the workspace, production builds, binding generation, packaging, security scans, release-candidate assembly, and other CPU-intensive jobs. The production VPS is for serving the live application and must not be used as a general-purpose build/test machine.

Cheap local checks are allowed when useful. No push, workflow dispatch/rerun, deployment, release, signing operation, or production change is authorized implicitly.

## Current source truth

NACC is a Windows-first Tauri 2 desktop application with a Rust privileged backend and React + TypeScript + Vite frontend. It has no runtime Node application server and does not expose a general shell/filesystem capability to the webview.

Mounted user-facing surfaces currently include:

- diagnostics summary
- Providers
- Role Matrix
- Run Console
- CI/CD Center
- Setup Wizard
- Workflow Inspector
- Worktree Manager
- Security & Audit Trail
- Quality & Review Center

The old statement that the frontend is “diagnostics only” is obsolete.

## Implemented core capabilities

### Providers and role routing

- Claude and Codex adapters are registered in the application.
- Provider detection and auth-presence checks are real subprocess-backed probes; credential/token contents are not read.
- Provider installations and capability snapshots are persisted.
- Role Matrix CRUD is wired to storage and routing refresh.
- Provider/model assignments fail closed when unsupported, unregistered, or absent from the latest verified capability snapshot.
- Provider-specific model/thinking/reasoning state is cleared or disabled when a provider change invalidates it.

### Workflow engine and worktrees

- Durable workflow templates and workflow runs are implemented.
- Workflow execution, pause, cancel, resume, approvals, checkpoints, and normalized events are wired through typed IPC.
- Per-node timeout/cancellation handling exists in the executor.
- Optional per-run worktree allocation is wired into run routing.
- Worktree release/quarantine logic preserves unsafe or unpushed work instead of silently deleting it.
- Startup reconciliation handles interrupted workflow/worktree state.

### GitHub / CI-CD

- `nacc-github` provides typed `gh` CLI integration for repository, branch, pull request, check run, workflow run, workflow job, failure evidence, artifact, environment, and pending-deployment data.
- The CI/CD Center is mounted in the frontend.
- Failed workflow reruns are approval/reason gated; there is no autonomous production approval or deployment bypass.
- Large GitHub numeric identifiers are transported as strings to avoid precision loss.

### Security and reliability already implemented

- Minimal Tauri capability surface; no webview shell/fs/http blanket access.
- Bounded subprocess execution with Windows Job Object containment and kill-on-drop behavior.
- Policy checks before Git/process execution, including force-push/protected-operation denial.
- Secret/token redaction before audit/evidence persistence.
- Storage worker panic propagation avoids silent thread death.
- Poisoned run-lease mutex handling is fail-closed/logged rather than blindly panicking.
- Audit records and workflow events use durable correlated storage.

### Release hardening

- `.github/workflows/desktop-build.yml` is explicitly non-distributable build evidence and does not publish a trusted release.
- `.github/workflows/signed-release-candidate.yml` is manual/protected and requires real Tauri updater signing plus Windows Authenticode material before producing candidate artifacts.
- Candidate generation does not create a public release or change the live updater channel.
- `docs/security/release-signing.md`, `docs/RELEASE-CHECKLIST.md`, and the threat model document the trust/sign-off path.

## In-flight production-hardening slice

The current uncommitted worktree contains additional hardening beyond the last verified remote run.

### Added backend contracts

1. `get_workflow_template`
   - returns full template metadata plus all persisted/built-in workflow nodes;
   - prevents an editor from re-saving only summary data and losing node configuration.

2. `list_worktree_leases`
   - read-only;
   - optional project filter;
   - optional active-only filter;
   - bounded result size.

3. `list_audit_records`
   - read-only;
   - newest-first bounded retrieval;
   - optional workflow-run filter;
   - default 100, maximum 500.

4. `list_quality_gate_results`
   - read-only;
   - persisted quality-gate evidence correlated to workflow/node runs and optional attempts;
   - optional workflow/node filters;
   - newest-first SQL-bounded retrieval.

5. `list_review_findings`
   - read-only;
   - persisted validated review findings correlated to workflow/node runs and optional attempts;
   - optional workflow/node filters;
   - newest-first SQL-bounded retrieval.

`crates/nacc-storage/src/audit.rs` also includes regression coverage for bounded newest-first retrieval.

Migration V8 adds append-only `quality_gate_results` and `review_findings` tables plus correlation/query indexes. `crates/nacc-storage/src/quality_review.rs` persists canonical `nacc-quality::QualityEvidence` and `nacc-review::ReviewFinding` data rather than inventing a duplicate domain contract.

Mounted read-only operational surfaces now include Workflow Inspector, Worktree Manager, Audit Log, and Quality & Review Center. The Quality/Review UI does not expose arbitrary shell/quality-gate execution or mutation controls.

### Binding-generation status

`src/bindings.ts` is generated by tauri-specta and must not be hand-edited. The local machine still lacks a usable MSVC linker for the pinned Windows target, but the Rust `1.96.0-x86_64-pc-windows-gnu` toolchain plus GCC is usable for focused Rust verification and authoritative binding export.

The authoritative exporter succeeded locally on 2026-09-19 with:

```powershell
cargo +1.96.0-x86_64-pc-windows-gnu run -p nacc-app -- --export-bindings
```

The generated contract now includes the mounted Workflow, Worktree, Audit, and Quality/Review surfaces. New IPC view models serialize 64-bit timestamps and durations as decimal strings so the JavaScript boundary does not lose integer precision and tauri-specta does not emit unsupported bigint-style number types.

CI remains the remote freshness gate and must still prove that regeneration produces no diff:

```powershell
cargo run -p nacc-app -- --export-bindings
git diff --exit-code -- src/bindings.ts
```

## Optional product backlog and hardening gaps

The items below remain real work, but they are **not blockers for the current
single-user local-only acceptance scope**. Unsupported functionality must stay
disabled/unavailable rather than be represented by dead controls.

- Workflow Designer mutation/editing frontend; the current Workflow Inspector is read-only.
- Destructive/mutating Worktree Manager controls; the current manager is intentionally read-only.
- Quality Gate / Review lifecycle completion. Typed per-node quality-gate declarations, graph validation, deterministic process-tree-contained execution after provider success, per-attempt evidence persistence, required-gate blocking/retry enforcement, durable storage, read-only IPC, and the read-only Quality & Review Center are implemented. Still incomplete are automatic review-finding lifecycle integration, dispositions/waivers, bounded repair-request lifecycle, richer execution metadata, and explicit rerun policy.
- Per-role network egress allowlisting. This remains useful defense-in-depth,
  especially before running untrusted repositories/plugins, but is not a local
  readiness gate for the current trusted single-user scope.
- MCP permission profiles. Keep current policy/permission enforcement; this
  finer-grained hardening remains backlog for the local-only target.
- Windows Credential Manager storage for NACC-owned secrets. Provider-native
  credentials remain external and unread; Credential Manager integration is
  required only if NACC begins owning secrets that need durable secure storage.
- Complete Usage/Quotas accounting.
- Complete Agent Sessions/Terminals management UI/runtime contract.

## Verification evidence

Last known remote green baseline before the present uncommitted hardening wave: GitHub Actions run `35260972351` on 2026-09-17, where all five then-existing CI jobs were green.

That run does **not** verify the present worktree.

Fresh bounded local evidence for the current hardening wave on 2026-09-19 includes:

- `cargo fmt --all -- --check` — passed.
- `nacc-quality` focused verification — passed, 11/11 tests, including timeout containment, OS-level process-tree cleanup coverage, and non-interactive stdin EOF behavior.
- `nacc-orchestrator` quality-gate verification — passed, 5/5 focused tests.
- `cargo +1.96.0-x86_64-pc-windows-gnu test -p nacc-storage quality_review --lib` — passed, 4/4 tests.
- `cargo +1.96.0-x86_64-pc-windows-gnu test -p nacc-storage a_database_left_at_v7_upgrades_to_v8_without_losing_data --lib` — passed.
- `cargo +1.96.0-x86_64-pc-windows-gnu run -p nacc-app -- --export-bindings` — authoritative tauri-specta export passed and regenerated `src/bindings.ts`.
- `cargo +1.96.0-x86_64-pc-windows-gnu test -p nacc-app quality_review::tests::evidence_limits_are_defaulted_and_bounded --lib` — passed.
- `npx tsc --noEmit` — passed.
- `npm test -- --run src/QualityReviewCenter.test.tsx src/AuditLog.test.tsx src/WorktreeManager.test.tsx src/App.test.tsx` — passed, 4 files / 16 tests.
- `git diff --check` — code/generated bindings are whitespace-clean; only the intentional Markdown hard-break spaces in the architecture handoff/status documents remain.

The current CI workflow is designed to provide the authoritative heavy-compute evidence:

- Rust formatting check
- frontend build + Vitest
- generated bindings freshness
- workspace clippy + tests on Windows MSVC
- workspace clippy + tests on Windows GNU parity

Current changes still require a fresh remote verification wave before any
distributed/public release claim. That remote release evidence is not a blocker
for personal local use.

## Local-only readiness contract

For the current single-user local-only target, readiness depends on the runtime
properties that protect the user's machine, repositories, credentials, and
durable data:

1. privileged execution remains in Rust behind typed IPC; the webview receives
   no general shell or unrestricted filesystem capability;
2. spawned process trees are contained and cancellation/timeout does not leave
   orphan descendants;
3. write-capable work is isolated and destructive worktree operations remain
   unavailable unless their privileged safety contract exists;
4. provider credentials are not copied/read by NACC, and durable evidence/logs
   remain redacted and bounded;
5. required deterministic quality gates fail closed and participate in normal
   retry/failure handling;
6. SQLite migrations/persistence preserve durable state across upgrades and
   restarts;
7. unsupported or unverified capabilities remain disabled rather than silently
   pretending to work.

Fresh 2026-09-19 evidence recorded above covers these current local-runtime
requirements sufficiently for the local-only target. Full-workspace CI remains
valuable regression evidence, but it is not a personal-local-use completion
gate.

The following are explicitly **future public-distribution requirements, not
local-use blockers**:

- protected signed release-candidate workflow;
- Authenticode installer signing/verification;
- clean-machine installer smoke testing;
- updater positive/negative signature testing and live update-channel trust;
- packaging/publication evidence and release-secret handling;
- exact-release provider contract certification intended for a distributed
  release;
- the master-plan release demonstration when used as a public-production
  certification gate.

## Rules for future implementation agents

- Do not touch `.zcode/`.
- Do not hand-edit `src/bindings.ts`.
- Do not invent provider/model support.
- Do not expose destructive worktree controls just because read-only lease IPC exists.
- Do not add Quality/Review mutation, arbitrary command execution, or enforcement controls until the privileged backend lifecycle and authorization contract exists.
- Do not use the production VPS for compilation/testing/processing.
- Do not push, dispatch workflows, deploy, release, or sign without explicit authorization.

**Local-use status: READY**

**Public/distributed release status: OUT OF SCOPE / NOT ASSESSED**

**Full product-roadmap completeness: PARTIAL** — optional features and hardening
items remain tracked above.

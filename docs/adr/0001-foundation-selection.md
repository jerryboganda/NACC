# ADR-0001: Foundation selection — fresh Tauri 2 workspace with an audited, MIT-attributed transplant of specific AgentPanel modules

**Status:** Accepted
**Date:** 2026-08-30
**Deciders:** NACC Phase 0 foundation audit
**Evidence:** `docs/audits/foundation-audit.md`, CI run
`https://github.com/jerryboganda/NACC/actions/runs/33314224962`

## Context

The master plan (§5) recommends AgentPanel (`GrillerGeek/AgentPanel`) as the
seed for NACC, conditional on passing the nine-criterion decision gate in
§5.4, and names a fresh Tauri 2 workspace with selective concept/code
transplant as the fallback if the gate fails. This ADR records that
decision with evidence, per §5.4's requirement and the build prompt's Phase
0 deliverables.

## Decision gate, scored against evidence

| # | §5.4 criterion | Verdict | Evidence |
|---:|---|:---:|---|
| 1 | Clean Windows build is reproducible | ✅ **Pass** | CI run succeeded end-to-end on `windows-latest`: `cargo fmt --check`, `cargo clippy -D warnings` (zero warnings), `cargo check`, `cargo test` (46/46 passed), `npm run build`, `npm test` (30 Vitest files), and `tauri build` all green in one attempt. Real NSIS installer downloaded and independently confirmed as a genuine PE32/Nullsoft archive (foundation-audit.md addendum). |
| 2 | License permits intended private modification/distribution | ✅ **Pass** | MIT, confirmed by direct read of `LICENSE`. Only obligation is retaining the copyright notice. Full transitive dependency tree (645 packages, from CI's `cargo metadata` run) contains no GPL/AGPL-family license — the sole near-hit (`r-efi`) is triple-licensed with MIT as a sufficient standalone option. |
| 3 | Terminal/process code isolatable behind a safe Rust interface | ✅ **Pass, with a caveat** | `pty.rs` is a single, well-bounded 330-line module with a clean internal interface (`PtyManager`, `pty_spawn`/`pty_write`/`pty_resize`/`pty_close`). The *code* is isolatable. Its *current IPC boundary* is not safe as-is — `pty_spawn` takes an open executable/cwd/env from the webview directly, which is exactly what master plan §12/§13.1 forbid. This is a solvable transplant problem (wrap the existing PTY engine behind a new policy-checked launch path), not a structural blocker — hence "pass, with a caveat" rather than a fail. |
| 4 | Frontend separable from core orchestration logic | ⚠️ **Marginal — this is the deciding criterion** | There is no "core orchestration logic" to separate *from* — AgentPanel has no workflow engine, no policy engine, no provider abstraction (confirmed absent by grep and by the module map in the audit). The frontend (`src/`) is a terminal-and-worktree panel built directly against 28 IPC commands with no intermediate domain layer. NACC's frontend must be a Role Matrix / Workflow Designer / Live Run Center / 17-page control center — a different application, not a themed reskin. Separating "frontend" from "orchestration" isn't possible because upstream never built the orchestration half to separate from. |
| 5 | Migrations/durable storage introducible without destructive rewrite | ✅ **Pass, trivially** | There is almost nothing to migrate *away from*: `store.rs` is 45 lines writing one JSON file; the rest is flat scrollback text files and browser `localStorage`. Introducing SQLite means adding a new subsystem, not rewriting a populated one. This criterion passes easily precisely because so little durable state exists yet — which is itself evidence for how much of the storage layer is net-new work regardless of which foundation is chosen. |
| 6 | Worktree behavior correct under cancellation/crash/Windows path edge cases | ⚠️ **Partially verified** | `git.rs`'s worktree add/remove has a real integration test (`git init` a temp repo, exercise the lifecycle) that passed in CI. The Windows lock-fallback on `remove_worktree` is a genuine, tested fix. **Not verified**: behavior under process crash mid-operation, or Windows long-path/UNC/junction edge cases — no test exercises these, and master plan §11 requires all of them. Process-tree cancellation itself is the weakest link (see #9 below) — `taskkill /T` provides no crash-survival guarantee. |
| 7 | Updater/security configuration hardenable | ✅ **Pass, but not free** | Nothing structurally prevents hardening — CSP can be set, a new signing keypair can be generated (`tauri signer generate`), capabilities can be tightened further than the already-minimal `default.json`. All confirmed straightforward changes, not architectural blockers. Must be done before any NACC-owned build ships, regardless of foundation choice. |
| 8 | Dependency tree maintainable | ✅ **Pass, strongly** | Zero clippy warnings under `-D warnings` across the whole crate; 645 resolved dependencies, zero copyleft; no floating-point version chaos (Cargo.lock present and used in CI). This is a genuinely clean, well-kept dependency tree for a 2.5-month-old project. |
| 9 | NACC modules addable without an untestable monolith | ❌ **Fail** | This is the load-bearing criterion, and where the gate actually turns. AgentPanel is a **single crate**, not a workspace — confirmed by direct read of `src-tauri/Cargo.toml` (no `[workspace]` table, no root `Cargo.toml`). The master plan's own architecture (§6) specifies ~19 independent crates (`nacc-domain`, `nacc-storage`, `nacc-orchestrator`, five `nacc-provider-*` crates, `nacc-policy`, `nacc-quality`, `nacc-review`, `nacc-worktree`, `nacc-git`, `nacc-github`, `nacc-secrets`, `nacc-observability`, `nacc-updater`, etc.) with an explicit dependency-direction rule ("provider crates depend on a common provider core... the orchestrator depends on abstractions, not concrete CLI parsers"). Retrofitting that boundary structure onto an existing single crate means either (a) converting it into a workspace and re-drawing every module boundary from scratch — at which point almost nothing about "starting from AgentPanel" survives except the file contents — or (b) building all 19 crates alongside the existing single crate and importing from it, which inverts the master plan's own dependency-direction rule (a leaf provider/process crate would depend on the monolith instead of the reverse). Neither path avoids becoming, in practice, a new workspace. |

**Result: 6 clear passes, 2 passes-with-caveats, 1 fail — and the fail is
the criterion the master plan itself treats as decisive** (§5.4's own
greenfield-fallback trigger list includes "an inability to... turning the
application into an untestable monolith," which is precisely #9).

## Decision

**Do not fork AgentPanel as the workspace foundation.** Create a fresh Tauri
2 workspace matching the master plan's `crates/` structure (§6) from the
first commit, and **transplant, individually, under MIT attribution**, the
specific modules identified in the foundation audit's §9 ("Retain the
technique, not the module" / "Retain the technique") table:

1. The ConPTY concurrent-spawn mutex fix from `pty.rs`, reimplemented inside
   `nacc-process` behind a policy-checked launch boundary (not exposing
   `pty_spawn`'s open parameter surface to the webview, per §12/§13.1).
2. The Windows worktree-removal lock-fallback pattern from `git.rs`, ported
   into `nacc-worktree`.
3. The `gh pr view --json`-based degrade-gracefully pattern from `gh.rs`,
   ported into `nacc-github`.
4. The scrollback path-traversal validation pattern (`is_safe_pane_id` in
   `scrollback.rs`) — a genuinely good defensive technique, reusable
   wherever NACC accepts a client-supplied identifier that becomes part of
   a file path.
5. `taskkill /T /F`'s direct-spawn-no-shell technique for `open_in_editor`
   (avoiding `cmd /C` escaping bugs) — the *pattern*, not the function,
   since NACC's editor-launch command needs its own policy check in front
   of it anyway.

Do **not** carry forward: `store.rs` (replaced by SQLite per §16), the
upstream Tauri updater signing key/endpoint (must be NACC's own), `csp:
null` (must be a real CSP), or `pty_spawn`'s unrestricted parameter surface
(must go through `nacc-policy`).

This is the master plan's own documented fallback (§3.2, §5.4's final
paragraph: *"Create a fresh Tauri 2 workspace and transplant only the
proven PTY, worktree, GitHub, and terminal concepts"*) — this ADR is
choosing the path the plan already anticipated, with evidence for why,
not deviating from the plan.

## Consequences

- NACC's Rust workspace starts clean, matching §6's crate boundaries from
  commit one, with no monolith-to-workspace migration debt.
- Real, tested Windows-specific knowledge (the ConPTY race fix, the
  worktree lock fallback, the console-flash suppression pattern via
  `CREATE_NO_WINDOW`) is preserved rather than rediscovered, at the modest
  cost of re-implementing rather than directly reusing five modules.
- MIT attribution is owed to Jason Robey (`GrillerGeek/AgentPanel`) in
  NACC's third-party notices for the transplanted techniques, even though
  no upstream file is imported unmodified.
- No ongoing upstream-merge relationship exists or is expected — there is
  no `docs/upstream-delta.md`-style tracking of a live fork, since NACC is
  not a fork. `docs/upstream-delta.md` (written alongside this ADR)
  documents the one-time transplant instead.
- Frontend is built fresh against the master plan's 17-page information
  architecture (§17) rather than adapted from AgentPanel's terminal-panel
  UI, consistent with criterion #4's finding that there is no shared
  "core" to separate a frontend from.

## Alternatives considered

- **Fork AgentPanel privately and restructure it into a workspace
  in-place** — rejected. The audit's own module map shows this converges
  on rewriting the crate boundaries, the storage layer, the process
  containment model, the CSP/updater config, and nearly the entire
  frontend. What would remain "forked" is a handful of files under new
  module paths — the practical outcome is identical to a fresh workspace
  with file-level ports, but with the ongoing overhead of a `git remote
  upstream` relationship to a 1-star, single-maintainer, 2.5-month-old
  project (`docs/upstream-delta.md` covers this scale finding) that offers
  little realistic prospect of useful future merges once NACC's shape has
  diverged this far.
- **Keep AgentPanel's single-crate layout and add NACC's crates as
  siblings that depend on it** — rejected. This inverts the master plan's
  own dependency-direction rule (§6: "provider crates depend on a common
  provider core... never on GUI components"); AgentPanel's crate mixes
  Tauri commands, PTY management, Git operations, and persistence in one
  package with no internal boundary to depend on selectively.
- **Do nothing with AgentPanel at all, write every Windows-specific piece
  from scratch** — rejected. The ConPTY concurrent-spawn bug in particular
  is the kind of non-obvious, hard-won fix worth preserving; discarding it
  risks NACC re-discovering the same intermittent Windows terminal stall
  during Phase 3.

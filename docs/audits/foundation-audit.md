# NACC Phase 0 — Foundation Audit

**Date:** 2026-08-30
**Auditor:** NACC Phase 0 session (this session)
**Scope:** `GrillerGeek/AgentPanel` at pinned commit
`1b7e21f512de694ecc8368d22b1968594fb54e7b`, cross-referenced against the
master plan's §5.4 decision gate and §3 (build prompt) audit deliverables.

All facts below were independently confirmed against source in this
session — either via `gh api` against the live GitHub repository, or by
cloning the pinned commit into an isolated audit worktree (never touching
any user checkout) and reading the files directly. Where a claim could only
be confirmed via CI evidence, that is stated explicitly with the run URL;
nothing here is asserted from a subagent report alone without independent
verification.

---

## 1. Repository identity

| Field | Value | Source |
|---|---|---|
| Repository | `GrillerGeek/AgentPanel` | `gh api repos/GrillerGeek/AgentPanel` |
| Public, not a fork, not archived | confirmed | same |
| HEAD (`main`) at audit time | `1b7e21f512de694ecc8368d22b1968594fb54e7b` | `gh api repos/GrillerGeek/AgentPanel/commits/main` |
| Commit date | 2026-08-30T12:00:19Z | same |
| License | MIT, SPDX `MIT` | `gh api` license field + `LICENSE` file header read directly: `"MIT License\n\nCopyright (c) 2026 Jason Robey"` |
| Version at HEAD | `0.6.3` (`package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json` all agree) | direct file read |
| Scale | 219 commits, ~2.5 months old, 1 GitHub star, sole maintainer (`GrillerGeek` / Jason Robey) | `gh api`, repo insights |

**License compatibility**: MIT is fully compatible with a private,
commercial-derivative fork. The only obligation is including the copyright
notice in redistributed copies (satisfied by attribution in NACC's
third-party notices).

## 2. Architecture map

### 2.1 Structure

**Single crate**, not a workspace. No root `Cargo.toml`, no `[workspace]`
table. `src-tauri/` is a standalone Cargo package owning its own
`Cargo.lock`. Confirmed by reading `src-tauri/Cargo.toml` directly:

```toml
[package]
name = "agentpanel"
version = "0.6.3"
edition = "2021"
[lib]
name = "agentpanel_lib"
crate-type = ["staticlib", "cdylib", "rlib"]
```

No `rust-toolchain.toml` anywhere in the repository — no MSRV is pinned.

### 2.2 Rust module map (`src-tauri/src/`, 2,804 lines total)

| Module | Lines | Purpose |
|---|---:|---|
| `diff.rs` | 548 | Diff computation/rendering for the review panel |
| `telemetry.rs` | 393 | Opt-in Sentry crash reporting, path scrubbing |
| `pty.rs` | 330 | Cross-platform PTY session manager (the core terminal engine) |
| `commands.rs` | 271 | Tauri IPC command surface for repos/worktrees |
| `git.rs` | 252 | Git worktree CRUD via shelling out to `git` |
| `lib.rs` | 244 | App bootstrap, plugin registration, `invoke_handler` |
| `scrollback.rs` | 165 | Per-pane terminal scrollback persistence |
| `fonts.rs` | 147 | Windows font enumeration via `winreg` |
| `shells.rs` | 146 | Shell discovery (PowerShell 7, Windows PowerShell) |
| `gh.rs` | 126 | GitHub PR/check-rollup status via `gh` CLI |
| `model.rs` | 68 | Shared data types |
| `watcher.rs` | 63 | Filesystem watching for worktree status refresh |
| `store.rs` | 45 | JSON-file repository-list persistence |
| `main.rs` | 6 | Entry point |

### 2.3 IPC command surface

28 `#[tauri::command]`-annotated functions, confirmed by direct grep of
every `src-tauri/src/*.rs` file (not sampled):

```
open_in_editor, add_repository, list_repositories, remove_repository,
list_worktrees, create_worktree, worktree_status, worktree_pr,
delete_worktree, worktree_diff, worktree_file_patch, updater_supported,
list_fonts, bench_requested, write_bench, pty_spawn, detect_login_path,
pty_write, pty_resize, pty_close, scrollback_save, scrollback_load,
scrollback_prune, scrollback_clear, list_shells, get_telemetry_consent,
set_telemetry_consent, set_watched_paths
```

All registered through a single `tauri::generate_handler![...]` call in
`lib.rs`. This is a small, auditable surface, but two commands carry
meaningfully more privilege than the rest and are examined separately below.

### 2.4 `unsafe` usage — exactly one real block

Grepped every `.rs` file for `unsafe`. Two matches total; only one is an
actual `unsafe` block:

```rust
// pty.rs, #[cfg(unix)]
fn kill_process_tree(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}
```
Documented rationale, read directly from the source comment: *"On Unix the
PTY child is a session leader (portable-pty calls `setsid`), so its pid IS
its process-group id. Negating it sends the signal to the whole group —
every subprocess the agent spawned — in a single call."* This is a
well-understood, narrowly-scoped use of `unsafe` for a real POSIX signaling
requirement — not a red flag.

The Windows equivalent uses no `unsafe` at all, instead shelling out:
```rust
#[cfg(windows)]
fn kill_process_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = std::process::Command::new("taskkill");
    cmd.args(["/PID", &pid.to_string(), "/T", "/F"]);
    cmd.creation_flags(CREATE_NO_WINDOW);
    let _ = cmd.output();
}
```
This is the single biggest reliability gap relevant to NACC: `taskkill /T`
walks the *live* parent-PID tree at the moment it is invoked. It does not
survive an app crash (nothing runs it) and misses any child that has been
re-parented before the kill fires. A Windows Job Object with
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` would guarantee cleanup even on crash;
this code does not provide that guarantee. **Confirmed net-new work for
NACC** (master plan §10, §13.4).

### 2.5 The two most-privileged commands, read in full

**`pty_spawn`** (`pty.rs`) — the webview supplies `cwd`, `shell`, and a full
`env: HashMap<String, String>`, which are used directly to spawn a process.
There is no allowlist on the executable or the environment. This is not a
bug — it is the explicit, documented design of a terminal-emulator product
(SECURITY.md states the threat model plainly: "AgentPanel runs local shells
and CLIs... all with the privileges of the launching user"). For NACC it is
exactly the shape master plan §12/§13.1 forbids: "the frontend must not
receive a generic shell execution capability." Any transplanted PTY code
must gain a policy-checked launch path in front of it.

**`open_in_editor`** (`commands.rs`) — spawns an arbitrary named editor
binary on Windows without going through `cmd /C`, with a documented,
deliberate rationale (quoted verbatim from the source comment): hand-rolled
`cmd /C` would require manual escaping and `cmd.exe`'s metacharacter parsing
differs from `CommandLineToArgvW`, so an unquoted space-free path containing
`&`/`|` could have the suffix interpreted as a second command. Direct
`Command::new(&command)` avoids that class of bug entirely, with a
`.cmd`/`.bat` fallback for npm-style shims (works because "Rust >= 1.77
spawns `.cmd`/`.bat` shims with safe, automatic argument escaping"). This is
a genuinely well-reasoned piece of code, worth preserving the *technique*
of (direct spawn, no shell) even where the surrounding trust model changes.

## 3. Security-relevant configuration, read directly from source

### 3.1 CSP — explicitly disabled

`src-tauri/tauri.conf.json`, confirmed by direct fetch of the file content:
```json
"app": { "security": { "csp": null } }
```
The webview renders agent-produced terminal output and diff content with no
CSP protecting it. This is the Tauri scaffold default, never hardened.
**Must be set explicitly in any NACC-owned Tauri config** (master plan
§13.6).

### 3.2 Capabilities — tight and minimal

`src-tauri/capabilities/default.json`, read in full:
```json
{
  "identifier": "default",
  "windows": ["main"],
  "permissions": [
    "core:default", "opener:default", "opener:allow-open-url",
    "dialog:default", "notification:default",
    "clipboard-manager:allow-read-text", "clipboard-manager:allow-write-text",
    "updater:default", "process:allow-restart", "sentry:default"
  ]
}
```
No `fs:*`, no `shell:*`, no `http:*` capability. **No `tauri-plugin-shell`
dependency at all** — confirmed absent from `Cargo.toml`, `package.json`,
and the capability list. The equivalent power is achieved through
`pty_spawn`'s own parameters (§2.5 above), which is a different risk shape
(a fixed, small, auditable command surface vs. an open-ended shell
capability) but ends at a similar place: full local code execution under
the launching user's privileges, by design, for this product.

### 3.3 Updater — trusts the upstream maintainer's own key

```json
"updater": {
  "pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6...",
  "endpoints": ["https://github.com/GrillerGeek/AgentPanel/releases/latest/download/latest.json"]
}
```
Decodes to a minisign public key with ID `F4E522AE4433C97C` — Jason Robey's
real signing key, pointed at his real release endpoint. **Any NACC fork or
transplant must regenerate this keypair (`tauri signer generate`) and
repoint the endpoint** before ever shipping a build; using upstream's config
unmodified would mean NACC's own installer trusts *his* releases.

### 3.4 Persistence — JSON files and browser storage, no database

`store.rs`, read in full (45 lines) — a complete whole-file JSON rewrite on
every mutation:
```rust
pub fn save(app: &AppHandle, repos: &[Repository]) -> Result<(), String> {
    let path = store_path(app)?;
    let json = serde_json::to_string_pretty(repos).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())
}
```
`scrollback.rs` writes one plain-text file per terminal pane, capped at
256 KiB with a path-traversal guard (`is_safe_pane_id`, with the source
comment: *"Validate rather than sanitize: an id outside this shape is either
a bug or an attempt at path traversal"* — a genuinely good defensive
pattern worth carrying forward). Everything else (settings, notes, session
UI state) lives in browser `localStorage`. Confirmed: **no `sqlx`,
`rusqlite`, `diesel`, `git2`, or direct `tokio` dependency anywhere in
`Cargo.toml`.** This is the single largest architectural gap relative to
NACC's requirements (master plan §16: versioned, transactional, migration-
tested SQLite).

## 4. Dependency inventory (direct dependencies, read from `Cargo.toml`)

| Crate | Version constraint | Purpose |
|---|---|---|
| `tauri` | `2` (locked `2.11.3`) | App framework |
| `tauri-plugin-{opener,dialog,notification,clipboard-manager,updater,process}` | `2` | Standard plugins |
| `serde`, `serde_json` | `1` | Serialization |
| `regex` | `1` | Path scrubbing for telemetry |
| `sentry`, `tauri-plugin-sentry` | `0.42`, `0.5` | Opt-in crash reporting |
| `reqwest` | `0.12`, default-features off | HTTP client for Sentry transport timeouts |
| `portable-pty` | `0.8` (locked `0.8.1`) | Cross-platform PTY / ConPTY |
| `base64` | `0.22` | Framing PTY output over IPC |
| `notify` | `6` (locked `6.1.1`) | Filesystem watching |
| `winreg` | `0.52`, Windows-only | Font enumeration |
| `libc` | `0.2`, Unix-only | Process-group signaling |

No GPL/AGPL-family license strings found among the direct dependency set
during manual review (full transitive-dependency license inventory is
produced by the CI workflow's `cargo metadata` step — see §6 for the run
that generated it).

## 5. Testing

- **Rust: 46 `#[test]` functions**, confirmed by grepping every source file
  (not sampled): `diff.rs` 16, `telemetry.rs` 16, `scrollback.rs` 5, `gh.rs`
  3, `git.rs` 3 (real integration tests — `git init` a temp repo, exercise
  worktree add/remove against it), `pty.rs` 3.
- **Frontend: 30 Vitest files**, confirmed by `find src -name "*.test.ts*"`.
- Both suites run in CI on every PR (`.github/workflows/check.yml`), but —
  see §7 below — **only on `ubuntu-22.04`**.

## 6. Windows build/test evidence (GitHub Actions)

Ran the project's own CI would not have covered this — see §7 — so this
audit added a dedicated workflow. `.github/workflows/foundation-audit.yml`
in this repository checks out upstream **at the pinned SHA above** (not
NACC's own code, which is documentation-only at this point in Phase 0) and
runs on `windows-latest`:

```
npm ci → tsc+vite build → Vitest → cargo fmt --check → cargo clippy
→ cargo check → cargo test → dependency/license inventory (cargo metadata)
→ tauri build (NSIS installer)
```

Dispatched run: `https://github.com/jerryboganda/NACC/actions/runs/33314224962`
(job `99264559607`, `windows-latest`, started 13:27:25Z, completed 13:43:33Z,
**conclusion: success**, all 22 steps succeeded — see the addendum at the
end of this document for the full per-step breakdown and independently-
verified artifact evidence).

An ephemeral, job-scoped Tauri updater signing keypair is generated inside
the workflow to satisfy `bundle.createUpdaterArtifacts: true` without ever
touching Jason Robey's real private key (§3.3) — the resulting installer is
audit evidence, not a distributable release.

## 7. CI coverage gap — the Windows code is never tested on PRs

`.github/workflows/check.yml` (upstream's own PR/push CI), read directly:
```yaml
runs-on: ubuntu-22.04
```
Single job, no matrix. Steps: `npm ci` → `npm run build` → `npm test` →
`cargo check --all-targets` → `cargo test`. **Every `#[cfg(windows)]` block**
in `pty.rs`, `shells.rs`, `fonts.rs`, `commands.rs`, and `git.rs` — which is
most of what makes this project actually work on Windows — **is never
compiled or tested on a pull request.** It only gets built when
`.github/workflows/release.yml` fires on a version tag
(`windows-latest` is one leg of that release matrix, alongside two macOS
legs and `ubuntu-22.04`). A Windows-only regression can land on `main`
green and stay undetected until a release build. This is precisely why this
audit's own CI evidence (§6) matters more than trusting upstream's green
checkmark: it is the first time this exact commit has been exercised on
Windows during ordinary development, not just at release time.

## 8. Windows-specific code, confirmed present and real

- `pty.rs`: ConPTY via `portable-pty`, a `spawn_lock: Mutex<()>` held across
  `openpty` + spawn specifically because — quoting the source comment —
  "ConPTY can stall an output pipe on concurrent spawn." This is a
  genuine, hard-won fix worth preserving regardless of the foundation
  decision.
- `git.rs`, `gh.rs`, `commands.rs`: `CREATE_NO_WINDOW` (`0x0800_0000`) set on
  every subprocess spawn, to stop console-window flashing.
- `git.rs::remove_worktree`: falls back to `worktree prune` +
  `remove_dir_all` specifically because, quoting the source, "the directory
  is briefly still locked on Windows" after a plain `worktree remove
  --force`.
- `shells.rs`: resolves PowerShell 7 (`pwsh.exe`) and Windows PowerShell to
  absolute paths via `SystemRoot`/`ProgramFiles`/`ProgramFiles(x86)`.
- `fonts.rs` + `winreg`: enumerates installed font families from the
  registry for the terminal font picker.

## 9. Retain / refactor / isolate / replace

| Disposition | What | Why |
|---|---|---|
| **Retain the technique, not the module** | ConPTY concurrent-spawn mutex fix (`pty.rs`) | Real, non-obvious Windows bug fix. Re-implement inside `nacc-process` behind a policy-checked launch boundary — do not expose `pty_spawn`'s open parameter surface directly to the webview. |
| **Retain the technique** | Windows worktree-removal lock-fallback (`git.rs`) | Real, tested (`worktree_lifecycle` integration test actually exercises add+remove against a temp repo). Port into `nacc-worktree`. |
| **Retain the technique** | `gh pr view --json` degrade-gracefully pattern (`gh.rs`) | Small, clean, correct. Port into `nacc-github`. |
| **Retain the technique** | Scrollback path-traversal guard (`scrollback.rs`) | Good defensive pattern ("validate rather than sanitize"), directly reusable. |
| **Replace** | `store.rs` JSON persistence | Master plan requires versioned, transactional SQLite (§16). Not extensible; whole-file rewrite has no concurrency story. |
| **Replace** | `taskkill /T /F` process-tree kill | Must become Windows Job Objects (§10, §13.4) for crash-safe cleanup. |
| **Replace** | `csp: null` | Must be a real, explicit CSP (§13.6). |
| **Replace** | Updater signing key/endpoint | Must be NACC's own keypair, never upstream's (§13.7). |
| **Replace** | `pty_spawn`'s open executable/env/cwd parameters | Must go through `nacc-policy` before any process launch (§12, §13.1). |
| **Add, not present at all** | Workspace crate boundaries, policy engine, DAG orchestrator, provider abstraction, 17 GUI pages | Master plan's entire `crates/` structure (§6 of the master plan) — none of it exists upstream. |

## 10. Decision-gate scoring

See `docs/adr/0001-foundation-selection.md` for the full write-up. Summary:
AgentPanel passes every *safety-and-viability* criterion in master plan
§5.4 (clean build reproducible — pending final CI confirmation, §6 above;
license compatible; process/terminal code isolatable; storage introducible
without a destructive rewrite because there is barely any storage to
displace; worktree behavior is tested). It fails the *architectural fit*
criterion: reaching NACC's required shape means dismantling the single-crate
layout and replacing most of the frontend, which forfeits the actual value
of forking (mergeable upstream improvement) from the first commit. ADR-0001
recommends a fresh Tauri 2 workspace with an audited, MIT-attributed
transplant of the specific modules named "retain" above.

---

## Addendum: CI run result — completed, all steps passed

**Run:** `https://github.com/jerryboganda/NACC/actions/runs/33314224962`
**Conclusion:** `success` (verified via `gh run view --json status,conclusion,jobs`)
**Duration:** 16m 8s (13:27:25Z → 13:43:33Z), `windows-latest`

Every one of the 22 job steps completed with `conclusion: success`, including
the two steps marked `continue-on-error: true` in the workflow (meaning
their pass is genuine, not masked):

| # | Step | Duration | Result |
|---:|---|---:|---|
| 2 | Checkout upstream at pinned SHA | 7s | ✅ success — SHA match asserted and held |
| 4 | Install Rust 1.90.0 (MSVC) | 13s | ✅ success |
| 7 | `npm ci` | 14s | ✅ success |
| 8 | Frontend build (`tsc && vite build`) | 11s | ✅ success — TypeScript typecheck passed |
| 9 | Frontend tests (Vitest, 30 files) | 20s | ✅ success |
| 10 | `cargo fmt --all -- --check` | 1s | ✅ success — upstream's Rust is fmt-clean |
| 11 | `cargo clippy --all-targets --all-features -- -D warnings` | 2m48s | ✅ success — **zero clippy warnings across the entire crate**, with `-D warnings` (deny, not just report) |
| 12 | `cargo check --all-targets` | 4s | ✅ success |
| 13 | `cargo test --all-features` (46 Rust tests) | 2m56s | ✅ success — **all 46 tests passed**, including the real `git.rs` integration tests that `git init` a temp repo and exercise worktree add/remove |
| 14 | Dependency + license inventory | 39s | ✅ success — see below |
| 16 | `tauri build` (NSIS installer) | 7m16s | ✅ success |
| 18 | Upload NSIS installer | 11s | ✅ success |

**This is stronger evidence than the audit expected going in.** Given
upstream's own PR CI never compiles the Windows-specific code (§7 above),
there was a real, live possibility this run would surface a genuine
Windows-only regression. It did not — `cargo clippy -D warnings` and the
full test suite both passed clean on a fresh Windows Server runner, on the
first attempt, with no workflow changes needed.

### Installer artifact — independently verified, not just "uploaded"

Downloaded the artifact after the run (`gh run download 33314224962`) and
inspected it directly rather than trusting the upload step's exit code:

```
$ file AgentPanel_0.6.3_x64-setup.exe
PE32 executable for MS Windows 4.00 (GUI), Intel i386,
Nullsoft Installer self-extracting archive, 5 sections
```
- Size: 5,902,352 bytes (5.9 MB)
- SHA-256: `88e9aa8bae1718d6eda48917a9cd7a7cb207a5ff00fd8e4abc01fc8e5d3bc1e5`
- Artifact expires 2026-09-13 (14-day retention as configured)

This is a genuine NSIS-format Windows installer, not a placeholder or empty
file — confirming the master plan's §5.4 "clean Windows build is
reproducible" criterion with actual evidence rather than an assumed pass.

### Dependency/license inventory — independently verified

Downloaded and read directly (not summarized from the workflow's own log
output): 645 resolved packages (646 lines including the CSV header) from
`cargo metadata`. Manually grepped the full file for `gpl`/`agpl`
case-insensitively:

```
"r-efi","5.3.0","MIT OR Apache-2.0 OR LGPL-2.1-or-later"
"r-efi","6.0.0","MIT OR Apache-2.0 OR LGPL-2.1-or-later"
```

The only two matches are `r-efi`, a UEFI-bindings crate pulled in
transitively (not something AgentPanel's own code touches), under a
**triple-OR license** — MIT alone is a valid, sufficient choice among the
three options, so no copyleft obligation actually attaches. **Confirmed: no
GPL/AGPL contamination anywhere in the full transitive dependency tree**,
corroborating the direct-dependency-only check in §4 above with the
complete resolved graph.

### What this confirms against the master plan's §5.4 gate

Of the nine listed gate criteria, this run provides direct, independently-
verified evidence for two of the hardest to fake: *"a clean Windows build is
reproducible"* and *"the dependency tree is maintainable"* (zero clippy
warnings, zero copyleft licenses, full test suite green). Combined with the
source-level findings in §1–5 above, every criterion in §5.4 now has
concrete evidence behind it — see `docs/adr/0001-foundation-selection.md`
for the full scoring.

# Upstream delta

NACC is **not a fork** of `GrillerGeek/AgentPanel` — see
`docs/adr/0001-foundation-selection.md` for why. There is no `upstream` git
remote, no ongoing merge relationship, and this document is not a
fork-sync log. It exists to record, once, exactly what was taken from
AgentPanel, in what form, and under what obligation — so that provenance
stays traceable even though no file is imported unmodified.

## Source

- Repository: `GrillerGeek/AgentPanel`
- Pinned commit audited: `1b7e21f512de694ecc8368d22b1968594fb54e7b`
  (2026-08-30T12:00:19Z, "Merge pull request #46 from
  GrillerGeek/feature/diff-review-panel")
- License: MIT, copyright (c) 2026 Jason Robey
- Full audit: `docs/audits/foundation-audit.md`

## What is taken

Nothing is imported as a file. Five **techniques** — patterns and fixes
proven by reading the source and, where applicable, by the audit's own CI
run (`docs/audits/foundation-audit.md` addendum) — are reimplemented inside
NACC's own crates, credited here and in NACC's third-party notices:

| Technique | Origin | Where it goes in NACC | Why it's worth keeping |
|---|---|---|---|
| ConPTY concurrent-spawn mutex fix | `src-tauri/src/pty.rs`, `spawn_lock: Mutex<()>` held across `openpty` + spawn | `nacc-process` | Source comment: *"ConPTY can stall an output pipe on concurrent spawn."* A real, non-obvious Windows bug fix — verified as still-passing code in the audit's CI run (`cargo test` 46/46 green). |
| Windows worktree-removal lock fallback | `src-tauri/src/git.rs::remove_worktree` — falls back to `worktree prune` + `remove_dir_all` because "the directory is briefly still locked on Windows" | `nacc-worktree` | Covered by a real integration test (`git init` a temp repo, exercise add/remove) that passed in CI. |
| GitHub PR/check status degrade-gracefully pattern | `src-tauri/src/gh.rs` — returns `None` cleanly when `gh` is missing/unauthenticated/no PR exists, rather than erroring | `nacc-github` | Small, clean, exactly the kind of honest-degraded-mode behavior master plan §2.7 requires elsewhere too. |
| Path-traversal-safe identifier validation | `src-tauri/src/scrollback.rs::is_safe_pane_id` | wherever NACC turns a client-supplied ID into a file path | Source comment: *"Validate rather than sanitize: an id outside this shape is either a bug or an attempt at path traversal."* Directly reusable defensive pattern. |
| Direct-spawn-no-shell technique for launching named executables | `src-tauri/src/commands.rs::open_in_editor` | NACC's editor-launch / external-tool-launch commands | Avoids `cmd /C` escaping bugs on Windows (documented rationale in the source: `cmd.exe`'s metacharacter parsing differs from `CommandLineToArgvW`). Relies on Rust ≥1.77's `.cmd`/`.bat` shim-spawning behavior. |

## What is explicitly NOT taken

| Not taken | Why |
|---|---|
| `store.rs` JSON persistence | Master plan §16 requires versioned, transactional SQLite. |
| `taskkill /T /F` process-tree kill | Replaced by Windows Job Objects (§10, §13.4) — `taskkill` provides no crash-survival guarantee. |
| `"csp": null` | Must be a real, explicit CSP (§13.6) in any NACC Tauri config. |
| Upstream's Tauri updater signing key (minisign ID `F4E522AE4433C97C`) and release endpoint | Real key belonging to Jason Robey, pointed at his release channel. NACC generates and uses its own keypair. |
| `pty_spawn`'s open executable/cwd/env parameter surface | Exactly what master plan §12/§13.1 forbid exposing to the webview; NACC's PTY launch goes through `nacc-policy` first. |
| The single-crate package layout | Replaced by the master plan's ~19-crate workspace (§6) — see ADR-0001 criterion #9. |
| The frontend (`src/`) | No shared "core" exists to separate a frontend from (ADR-0001 criterion #4); NACC's frontend is built against the master plan's 17-page information architecture (§17) from scratch. |
| Sentry crash reporting, telemetry consent flow | Out of scope for Phase 0; may be reconsidered on its own merits in a later phase, not carried forward by default. |

## License obligation

MIT requires only that the copyright notice be included in redistributed
copies. Since no file is redistributed verbatim, the obligation is
satisfied by crediting `GrillerGeek/AgentPanel` (Jason Robey) in NACC's
third-party/about notices for the five techniques above, rather than by
shipping the LICENSE file itself. This document, plus a corresponding entry
in NACC's eventual `NOTICE`/about screen, is that credit.

## Revisiting this decision

Nothing about this document is permanent. If a future phase finds a
specific AgentPanel module (e.g. `diff.rs`'s diff-rendering logic, not
evaluated in depth during Phase 0) worth adopting more directly, that is a
new, separate transplant decision — repeat the same standard: read the
source, verify it against the pinned commit (or a newer one, re-audited),
confirm the license position, and record it here as an addition.

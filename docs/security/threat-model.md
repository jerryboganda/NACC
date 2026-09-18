# NACC threat model (master plan §13, build prompt §12) — implemented state

Scope: the NACC desktop app as built on 2026-09-19 for a single-user,
local-only deployment. Each entry names the
threat and the control that exists in code today, or states the gap
honestly.

| Threat | Control in this build |
|---|---|
| Agent writes outside its assigned directory | Explicit workspace per run, validated absolute+existing; optional per-run git worktree leases (S16) isolate all writes; policy engine `check_write` refuses protected paths |
| Agent runs destructive git operations | Policy engine denied-fragment list (`push --force`, `branch -D main`) holds even under danger mode; approval gates on integrate/push nodes (S12.2) |
| Malicious repository instructions (prompt injection) | Permission profiles narrow per node; read-only roles cannot write; review is cross-provider by template design; quality gates decide success, not model claims (S27.19) |
| Secret leakage into durable records | `nacc-secrets` value + token-shape redaction at the record boundary (S13.5); adapters summarize tool inputs by keys only; provider credential stores are presence-checked, never read |
| Orphaned agent processes | Windows Job Objects with kill-on-close; executor cancels via the adapter on timeout; run cancellation kills the whole in-flight session tree (S13.4) |
| Dirty worktree destruction | Worktree release quarantines anything preserved instead of deleting (S16) |
| Webview shell escape | No shell in the webview; typed IPC only (tauri-specta); no runtime Node server |
| Updater compromise | `tauri-plugin-updater` verifies against the public key bundled in `tauri.conf.json`; build-only CI uses an explicitly ephemeral key and discards those updater artifacts. Public updater/signing certification is future-distribution scope and does not gate personal local use. If public distribution is reintroduced, the protected signed-candidate workflow plus clean-machine/updater negative testing remains required (`docs/security/release-signing.md`). |
| Log/crash-report leakage | Tracing to local app-log dir only; redaction pass available at the record boundary; no telemetry |
| Danger-mode abuse | Per-run, expiring grants; expiry enforced at read time; cannot self-extend; protected paths and denied fragments unaffected |

Known hardening gaps (not implemented): network egress allowlisting per role,
MCP permission profiles, and Windows Credential Manager integration for
NACC-owned secrets. These are not silently assumed to be covered by another
control. Under the current trusted single-user/local-only acceptance scope they
are non-blocking backlog, while the existing policy, redaction, credential
non-extraction, typed IPC, process-containment, and worktree-safety controls
remain mandatory. Reassess these gaps before treating untrusted
repositories/plugins or public distribution as supported scope.

# Codex CLI adapter fixtures

| File | Provenance |
|---|---|
| `invocations.json` | `--version` output in the form the installed CLI reports it, per `docs/provider-adapters/codex.md` (`codex-cli 0.149.1`). |
| `jsonl-run.json` | **Documented-shape sample, not captured output.** The contract doc states plainly that no live `--json` event shape was exercised in Phase 0 ("Any live JSONL event shape from `--json` — not exercised"). Each line follows Codex's own documented event shape. |

Codex's JSONL vocabulary is **not** ACP: the Phase 0 audit grepped the whole
installed help output for `acp`/`agent-client` and found zero matches. This
adapter therefore maps Codex's own event names, and ignores unrecognized
event types at TRACE level instead of guessing what they mean.

`"args": [..., "*"]` is an argument pattern: `*` matches exactly one
argument of any value.

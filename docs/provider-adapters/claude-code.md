# Provider adapter contract — Claude Code CLI

**Status:** Verified against the installed binary on 2026-08-30. Primary evidence is
`claude --help` captured live (below); vendor docs at `code.claude.com/docs/en/`
were used as corroboration, not as the primary source, because the installed
CLI's flag surface is ahead of what the published docs describe (see
"Discrepancies from published docs").

| | |
|---|---|
| Installed version | `2.1.215 (Claude Code)` |
| Latest on npm (`@anthropic-ai/claude-code`) | `2.1.251` (per docs-pass research; not independently re-verified here) |
| Resolved path | `C:\Users\Dr Faisal Maqsood PC\.local\bin\claude.exe` |
| Auth store | `~/.claude/.credentials.json` (present; contents not read) |

## Non-interactive execution

```
claude -p "<prompt>" [options]
```
`-p, --print` — "Print response and exit (useful for pipes)." The workspace
trust dialog is skipped in non-interactive mode; settings files that fail
validation are silently ignored (no error dialog).

## Structured output

- `--output-format <text|json|stream-json>` — only meaningful with `-p`.
- `--input-format <text|stream-json>` — realtime streaming input, `-p` only.
- `--include-partial-messages` — token-level streaming chunks, requires `-p` +
  `stream-json`.
- `--json-schema '<JSON Schema>'` — constrains the final result to a schema.
  Example from `--help`: `{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}`.
- `--forward-subagent-text` — forwards subagent text/thinking blocks with
  `parent_tool_use_id` set (requires `-p` + `stream-json`).

## Model selection

`--model <model>` — "Provide an alias for the latest model (e.g. `'fable'`,
`'opus'`, or `'sonnet'`) or a model's full name (e.g. `'claude-fable-5'`)."

**This is directly relevant to the user's stated orchestrator setup**: the
installed CLI's own help text confirms `claude-fable-5` as a valid full model
name, corroborating that Fable 5 is a real, addressable model — separately
from whether it is exposed through a Copilot Enterprise BYOK route (see
`copilot.md`).

## Reasoning effort — CORRECTION to the docs-only pass

**`--effort <level>` is a real top-level flag** on the installed binary:
`low, medium, high, xhigh, max`. An earlier documentation-only research pass
concluded Claude Code only exposed effort via an in-session `/effort` slash
command with no top-level flag; that is contradicted by this machine's
`claude --help` output. Treat the flag as authoritative. Canonical-scale
mapping for NACC's switch 4 (§10.1 of the master plan) is close to 1:1:
`Off/Minimal→low, Low→low, Medium→medium, High→high, XHigh→xhigh, Max→max`
(`Off`/`Minimal` both collapse onto Claude's floor since it has no explicit
"none" level — this must render as a visible, audited mapping, never silent).

## Permission modes

`--permission-mode <mode>` — choices: `acceptEdits`, `auto`, `bypassPermissions`,
`manual`, `dontAsk`, `plan`.
Related: `--dangerously-skip-permissions` / `--allow-dangerously-skip-permissions`.

## Tool policy

`--allowedTools`/`--allowed-tools` and `--disallowedTools`/`--disallowed-tools`
— comma/space-separated tool name lists, e.g. `"Bash(git *)" Edit`.
`--tools <tools...>` — restrict the built-in tool set; `""` disables all,
`"default"` enables all.

## Session resume

- `-r, --resume [value]` — resume by session ID, or open an interactive picker.
- `-c, --continue` — continue the most recent conversation in the current directory.
- `--session-id <uuid>` — pin a specific session ID.
- `--fork-session` — resume into a *new* session ID instead of reusing the original.
- `--no-session-persistence` — disable persistence entirely (only with `-p`).

## Budget / fallback

- `--max-budget-usd <amount>` — caps spend, `-p` only.
- `--fallback-model <model>` — comma-separated fallback list, retried at the
  start of each user turn, `-p` only.

## Cancellation — the one provider with a documented contract

Per `code.claude.com/docs/en/headless` (docs pass, not re-verified live this
session): SIGTERM → exit code 143, leaves the current turn unfinished and
resumable, terminates the process tree of any running Bash commands, runs
`SessionEnd` hooks. SIGINT ends the current turn cleanly instead of aborting
mid-flight. **No other installed CLI documents an equivalent contract** — see
the audit's cross-provider note on cancellation.

## ACP (Agent Client Protocol)

**Not exposed by the `claude` binary itself.** No `--acp` flag or `acp`
subcommand appears anywhere in `claude --help`. Verified live: `npm view
@zed-industries/claude-code-acp` and `npm view claude-code-acp` both resolve
to real, currently-published bridge packages
(`@zed-industries/claude-code-acp@0.16.2`, `claude-code-acp@0.1.1`/`cc-acp`)
that wrap the `claude` CLI to speak ACP. If NACC adopts ACP as a transport for
Claude, it goes through one of these bridges, not the CLI directly — a real
extra moving part (dependency, versioning, trust) that Copilot and Gemini's
native `--acp` flags don't require.

## What was NOT verified this session

- Exact behavior of `--effort` combined with a model that doesn't support a
  given level (block vs. clamp vs. error) — not exercised live.
- Whether `--output-format stream-json` events map cleanly onto the master
  plan's normalized event vocabulary (§8.2) — needs a real run, deferred to
  Phase 4/5.
- `claude doctor` was not run (would touch local state beyond a pure probe).

## Raw evidence

Full `claude --help` output captured verbatim in this session's scratchpad at
`provider-probes/claude.txt` (234 lines). Not reproduced in full here; quoted
above are the fields NACC's adapter contract actually needs.

# Provider adapter contract — GitHub Copilot CLI

**Status:** Verified against the installed binary on 2026-08-30. Primary
evidence is `copilot --help` and its help topics (`providers`, `environment`,
`logging`) captured live, plus a real ACP handshake attempt against the
native `copilot.exe`. **This adapter matters more than the others in this
document set**: the user runs Copilot Enterprise and intends to use it
off-and-on as the main orchestrator brain, routed to Claude Fable 5 via BYOK.

| | |
|---|---|
| Installed version | `GitHub Copilot CLI 1.0.82` (current, per docs pass) |
| Resolved path (npm shim) | `C:\Users\Dr Faisal Maqsood PC\AppData\Roaming\npm\copilot.cmd` |
| Native binary | `...\npm\node_modules\@github\copilot\node_modules\@github\copilot-win32-x64\copilot.exe` |
| Auth store | `~/.copilot` (present); logs at `~/.copilot/logs/` |
| GitHub account | `jerryboganda`, `gh auth status` shows `copilot` scope, classic PAT |

## Correction to the earlier docs-only pass

GitHub's published *programmatic reference* omits most of this CLI's real
surface. A documentation-only research pass concluded Copilot had no
structured/machine-readable output at all. That is **wrong** — `copilot
--help` on the installed 1.0.82 binary proves it has the richest surface of
any adapter in this set. Every claim below is anchored in the installed
binary's own `--help` output, not the published reference. Where the
published docs and the binary disagree, this document follows the binary.

## Non-interactive execution

```
copilot -p "<prompt>" [options]
copilot -i "<prompt>"     # interactive mode, auto-executes this prompt first
```
`-s, --silent` — "Output only the agent response (no stats)" — for scripting.

## Structured output

- `--output-format <text|json>` — `json` is **"JSONL, one JSON object per
  line."** This directly satisfies the master plan's normalized-event
  requirement (§8.2).
- `--stream <on|off>` — explicit streaming toggle.
- `--usage-output-file <file>` — final usage statistics as JSON (§17.13,
  Usage and Quotas).
- `--share[=path]` / `--share-gist` — markdown session transcript, useful for
  human-readable audit trails but not a structured event source.

## ACP (Agent Client Protocol) — real flag, live-tested, blocked at auth

`--acp` — **"Start as Agent Client Protocol server."** This is a genuine,
documented top-level flag on the installed binary.

**Live handshake attempt** (this session): spawned the native `copilot.exe
--acp` directly (bypassing the `.cmd`→node→spawnSync hop chain, which
produces zero stdout/stderr on its own and is a dead end for probing —
invoke the native `.exe` under `node_modules\@github\copilot\node_modules\
@github\copilot-win32-x64\` directly), wrote a spec-correct ACP `initialize`
request to stdin (`protocolVersion: 1`, `clientCapabilities`, `clientInfo`
per `agentclientprotocol.com/protocol/initialization`), and read
`~/.copilot/logs/process-*.log` for ground truth (the process itself
produced no stdout in either attempt). Two concrete findings:

1. **Enterprise policy fail-closed.** The log shows: `bypass-permissions mode
   DISABLED by enterprise policy (fail-closed: policy could not be
   determined) — /allow-all and permission escalation are now blocked`. On
   this Enterprise account, `--allow-all`/`--yolo` may be unavailable until
   the org's managed policy resolves successfully. This is a real constraint
   on the "Autonomous Worktree" permission profile (§12.1) when the acting
   provider is Copilot Enterprise — the GUI must be able to show this as a
   live, provider-reported restriction, not silently retry or hide it.
2. **`No authentication information found`** after ~24s, before any ACP
   `initialize` response was ever written to stdout. ACP mode requires its
   own authentication, separate from `gh`'s. Confirmed directly: exporting
   `GH_TOKEN` from the already-authenticated `gh` CLI (`jerryboganda`,
   classic PAT) produced an immediate, precise rejection:
   ```
   Error: Classic Personal Access Tokens (ghp_) are not supported by Copilot.
   The GH_TOKEN environment variable contains a classic PAT.
     • Replace the token in GH_TOKEN with a fine-grained PAT
     • Unset GH_TOKEN and run 'gh auth login' to authenticate
   ```
   **Copilot CLI auth is not interchangeable with `gh`'s** — a fine-grained
   PAT or `copilot login` is required. This directly informs the master
   plan's native-credentials rule (§8.4): NACC cannot assume one GitHub
   auth serves every GitHub-adjacent tool.

**Deliberately stopped here.** Completing the `initialize`↔`result` round
trip needs either `copilot login` (an interactive login flow, explicitly out
of scope for this plan) or minting a new fine-grained PAT (a new credential
the user hasn't asked for). The flag, the server process, and its startup
sequence (OpenTelemetry init, session indexing, managed-settings resolution)
are all confirmed real; the wire-level `initialize`/`result` exchange itself
is **not yet verified** and needs a real authenticated session in Phase 4.

## Reasoning effort — exact match to the canonical scale

`--effort, --reasoning-effort <level>` — choices: `none, minimal, low,
medium, high, xhigh, max`. **This is a 1:1 match to the master plan's
canonical reasoning scale (§10.1)** (`Auto` is the only value without a
direct CLI equivalent — map it to omitting the flag, which the help text
says lets Copilot pick automatically via `--model auto`-style defaulting).
No other installed CLI matches this cleanly; see the audit's cross-provider
comparison table.

## Model selection

`--model <model>` — "Set the AI model to use (use 'auto' to let Copilot pick
automatically)." Example in `--help`: `copilot --model gpt-5.4`.

**Not yet verified**: whether `claude-fable-5` (or an equivalent alias) is
present in this Enterprise account's model catalog, and how org/enterprise
model policy gates it. No model-listing command was found in `--help`
(no `copilot models` or `--list-models`); the in-session `/model` command is
the only discovery path documented, which this session did not exercise
(would require an interactive session). **This is the single most important
open item for the user's stated orchestrator use case and should be the
first live check in Phase 4.**

## BYOK — officially documented, not a workaround

`copilot help providers` is a first-class, officially documented help topic
(quoted in full in the audit). Key points:

- Activated by setting `COPILOT_PROVIDER_BASE_URL`; GitHub auth is not
  required once BYOK is active.
- `COPILOT_PROVIDER_TYPE` ∈ `openai` (default) | `azure` | `anthropic`.
- **An `anthropic` provider type exists as a first-class option**, with a
  documented example using `claude-sonnet-4-20250514` directly against
  `https://api.anthropic.com`. This is the concrete mechanism for routing
  Claude models through Copilot's engine.
- `COPILOT_MODEL` sets both the model ID and wire model in the simple case;
  `COPILOT_PROVIDER_MODEL_ID` / `COPILOT_PROVIDER_WIRE_MODEL` split them when
  they differ (fine-tunes, Azure deployments).
- `COPILOT_PROVIDER_MAX_PROMPT_TOKENS` / `_MAX_OUTPUT_TOKENS` override token
  limits; resolution order is manual env vars → built-in model catalog →
  defaults.
- `COPILOT_PROVIDER_HEADERS` — custom headers sent only to the BYOK
  endpoint, never to GitHub's own services.

This is a genuinely different (and better) path than treating Copilot as a
routing layer only for GitHub-hosted models — it means a single Copilot CLI
adapter in `nacc-provider-core` can also serve as one of NACC's OpenAI/
Anthropic-compatible gateway integrations (§9.5) when configured this way,
though the master plan scopes that role to OpenCode. Worth an explicit note
in ADR-0002 rather than silently picking one.

## Permission / tool policy

- `--allow-tool[=tools...]` / `--deny-tool[=tools...]` — pattern syntax
  confirmed from `--help`: `shell(git:*)`, `write`, `MyMCP(denied_tool)`.
  **Deny takes precedence over allow, even under `--allow-all`.**
- `--allow-all` / `--yolo` — equivalent to `--allow-all-tools
  --allow-all-paths --allow-all-urls`. **Subject to the enterprise
  fail-closed restriction above.**
- `--mode <interactive|plan|autopilot>`, `--plan`,
  `--assisted-approval` (routes approvals through a "safety judge"; requires
  `--experimental` or a feature flag).
- `--secret-env-vars=VAR,...` — stripped from shell/MCP env *and* redacted
  from output (§13.5).

## Session resume

`--session-id <uuid>`, `--resume[=id|prefix|name]`, `--continue`.

## Observability

`--log-dir` (default `~/.copilot/logs/`), `--log-level
<none|error|warning|info|debug|all|default>`. A documented `monitoring` help
topic covers full OpenTelemetry configuration
(`COPILOT_OTEL_ENABLED`, `OTEL_EXPORTER_OTLP_ENDPOINT`, mTLS cert vars,
etc.) — directly usable for NACC's own observability layer (§22) rather than
NACC having to build a separate telemetry bridge for this provider.

## Cancellation

**Unverified.** No signal contract found in `--help`; not exercised live
this session beyond killing the ACP probe process, which is not equivalent
to testing graceful cancellation of an in-flight turn.

## What was NOT verified this session

- The actual ACP `initialize`/`result` payload shape (blocked on auth, by
  design — see above).
- Model catalog contents for this Enterprise account, and whether Fable 5 is
  in it.
- `--assisted-approval`'s actual judge behavior.
- Exact enterprise-policy resolution flow (what makes `serverFetchFailed`
  vs. a resolved policy, and whether it resolves once real GitHub auth is
  present in a full session).

## Raw evidence

Full `copilot --help`, `copilot help providers`, `copilot help environment`,
`copilot help logging` captured verbatim in this session's scratchpad at
`provider-probes/copilot-help.txt`. ACP probe transcripts and the real
`~/.copilot/logs/process-*.log` startup sequence were read directly (not
copied verbatim into this doc; timestamps and exact log lines quoted above
are transcribed faithfully from that file).

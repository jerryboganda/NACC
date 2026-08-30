# Provider adapter contract — OpenAI Codex CLI

**Status:** Verified against the installed binary on 2026-08-30. Primary
evidence is `codex --help`, `codex exec --help`, `codex app-server --help`,
and `codex mcp-server --help` captured live; `learn.chatgpt.com/docs/*` was
used as corroboration.

| | |
|---|---|
| Installed version | `codex-cli 0.149.1` |
| Latest release (GitHub, docs pass) | `rust-v0.151.0`, 2026-08-29 |
| Resolved path | `C:\Users\Dr Faisal Maqsood PC\AppData\Roaming\npm\codex.ps1` |
| Auth store | `~/.codex/auth.json` (present; contents not read) |

## Non-interactive execution

```
codex exec [OPTIONS] [PROMPT]
codex exec [OPTIONS] <COMMAND>   # resume | fork | review | help
```
`exec` (alias `e`) is the documented non-interactive entry point.
`--full-auto` does **not** appear anywhere in the installed CLI's help output
— it is deprecated per the docs pass ("prefer `--sandbox workspace-write`");
treat any reference to it in older material as stale.

## Structured output

- `--json` — "Print events to stdout as JSONL."
- `--output-schema <FILE>` — path to a JSON Schema file describing the final
  response shape.
- `-o, --output-last-message <FILE>` — writes just the final message to a file.

## Model selection

`-m, --model <MODEL>` (also `-c model="..."` via config override). `--oss` /
`--local-provider <lmstudio|ollama>` select a local/open-source provider
instead of the default one.

## Reasoning effort

No dedicated top-level flag on the installed binary — confirmed absent from
`codex exec --help`. Set via config override:
```
-c model_reasoning_effort=<minimal|low|medium|high|xhigh>
```
No `none`/`off` and no `max` value exist; NACC's canonical `Max` must map to
`xhigh` **visibly** (§10.1: never silently downgrade), and `Off`/`Minimal`
both collapse onto `minimal`.

## Sandbox / approval

- `-s, --sandbox <read-only|workspace-write|danger-full-access>`
- `-a, --ask-for-approval <APPROVAL_POLICY>` — installed CLI's help lists
  `on-request` and `never` as the enumerated values (top-level `codex --help`,
  not `codex exec --help` specifically — approval policy is a top-level
  concept). The docs-pass third value `untrusted` was not observed in the
  live `--help` text; treat as **unverified** until confirmed against a real
  run or the config schema.
- `--dangerously-bypass-approvals-and-sandbox` — "EXTREMELY DANGEROUS. Intended
  solely for running in environments that are externally sandboxed." A real
  open GitHub issue (#4565, per docs pass) reports this mode can stall forever
  on a forced tool approval — **any adapter using this must run under an
  external watchdog timeout**, not trust Codex's own timeout handling.
- `--approve-for-me` — routes approval requests through automatic review
  using the workspace-write sandbox (present on this version; not covered by
  the docs pass at all — a newer addition worth revisiting before Phase 5).

## Session resume

`codex exec resume [SESSION_ID | --last]`, `codex exec fork <SESSION_ID>` —
confirmed as real subcommands under `exec`, not just top-level commands.
Also present at the top level: `resume`, `fork`, `queue`, `archive`, `delete`,
`unarchive`, `migrate-rollouts`.

## Experimental server modes — real, but their contracts are NOT ACP

Three genuinely-shipping-but-experimental subcommands, confirmed present:

- `app-server` — own transport (`--listen stdio://|unix://|ws://IP:PORT|off`,
  or `--stdio`), own websocket auth modes (`capability-token`,
  `signed-bearer-token`), `daemon`/`proxy`/`generate-ts`/`generate-json-schema`
  sub-subcommands. This is a **first-party, Codex-specific JSON-RPC-shaped
  protocol**, not Agent Client Protocol — no ACP framing or method names
  appear anywhere in its help output.
- `mcp-server` — starts Codex itself as an MCP server over stdio (Codex as
  the *tool provider*, not as an ACP agent).
- `exec-server` — listed as `[EXPERIMENTAL]`, no further detail captured.

**No `--acp` flag or `acp` subcommand exists anywhere in the installed CLI.**
Grepped the full captured help text for `acp`/`agent-client`/`agent.client` —
zero matches. Codex is the one installed CLI with no ACP path at all, native
or bridged, as of this session.

## Cancellation

**Unverified.** No signal-handling contract found in the docs pass; not
exercised live this session (would require a real run). The open issue about
`--dangerously-bypass-approvals-and-sandbox` stalling (above) is the only
concrete cancellation-adjacent evidence gathered.

## What was NOT verified this session

- `-a` approval-policy's exact enumerated values under `codex exec`
  specifically (only confirmed at top level).
- Any live JSONL event shape from `--json` — not exercised.
- `app-server`'s actual RPC method surface — help text only, no live probe.

## Raw evidence

Full help output for `codex --help`, `codex exec --help`, `codex app-server
--help`, `codex mcp-server --help` captured verbatim in this session's
scratchpad at `provider-probes/codex.txt` and `provider-probes/codex-servers.txt`.

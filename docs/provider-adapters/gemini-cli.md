# Provider adapter — Google Gemini CLI (bonus probe, not one of the five required providers)

**Status:** Installed and runs, but individual free-tier access is
Google-side sunset in favor of Antigravity. Captured as evidence directly
relevant to the Antigravity adapter decision (see `antigravity.md`), not
because Gemini CLI is itself a planned NACC provider.

| | |
|---|---|
| Installed version | `0.40.0` |
| Resolved path | `C:\Users\Dr Faisal Maqsood PC\AppData\Roaming\npm\gemini.cmd` |

## Flag surface (from live `gemini --help`)

Non-interactive: `-p, --prompt`. Structured output:
`-o, --output-format <text|json|stream-json>`. Model: `-m, --model`.
Permission/approval: `-y, --yolo`, `--approval-mode
<default|auto_edit|yolo|plan>`, `-s, --sandbox`. Resume:
`-r, --resume <"latest"|index>`, `--list-sessions`, `--delete-session`.
Worktree: `-w, --worktree` — Gemini CLI can start itself in a new git
worktree directly, a capability none of the other four adapters expose.

**`--acp` is a real, native boolean flag** — *"Starts the agent in ACP
mode"* — with a deprecated alias `--experimental-acp`. This corrects an
earlier draft of the audit's own live-testing conclusion ("today, only
Copilot speaks ACP among installed CLIs"): **Copilot and Gemini both have
genuine native `--acp` support**; Claude needs a third-party bridge package;
Codex has no ACP surface at all. See `docs/adr/0002-provider-transport.md`
for the full per-provider scoring.

## Live ACP probe result — the important finding

Spawned `gemini.cmd --acp` directly and sent the same spec-correct ACP
`initialize` request used for the Copilot probe. Result, captured from
stderr:

```
Error authenticating: IneligibleTierError: This client is no longer supported
for Gemini Code Assist for individuals. To continue using Gemini, please
migrate to the Antigravity suite of products: https://antigravity.google
    at throwIneligibleOrProjectIdError (...@google/gemini-cli/bundle/chunk-SZYCJREE.js:272870:11)
    tierId: 'free-tier',
    reasonMessage: 'This client is no longer supported for Gemini Code Assist
      for individuals. To continue using Gemini, please migrate to the
      Antigravity suite of products: https://antigravity.google',
    ineligibleTiers: [ reasonCode: 'UNSUPPORTED_CLIENT' ]
```
Exit code `55`, ~4.4 seconds after spawn.

This is a first-party, unambiguous statement from Google's own client: free
individual-tier access to Gemini Code Assist (which Gemini CLI authenticates
against) is being turned away and redirected specifically to Antigravity.
This is the primary evidence behind `antigravity.md`'s conclusion that
Antigravity is Gemini CLI's direct successor product, not a separate
alternative — which matters directly for the user's stated intent to route
"all AI usage" through the Google AI Pro / Antigravity quota by default.

## Structural pattern worth generalizing

Both the Copilot and Gemini `--acp` probes failed at their own
provider-specific auth/eligibility gate **before** either process ever wrote
an ACP `initialize` response to stdout. Neither failure was a protocol-level
problem — both were "this credential isn't good enough for this specific
mode," surfaced through completely different mechanisms (a silent 24-second
hang ending in a log-file-only error for Copilot; an immediate, loud
stderr exception for Gemini). **Conclusion for `nacc-provider-core`'s
contract-test design (master plan §6)**: an ACP adapter contract test cannot
be written as "spawn the process and expect a JSON-RPC response" in the
abstract — it needs a real, currently-valid, mode-appropriate credential for
each provider, and the *absence* of one needs to be treated as its own
distinct, testable failure mode (not lumped in with "malformed output" or
"timeout").

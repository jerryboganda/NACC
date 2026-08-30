# ADR-0002: Provider transport — per-provider bespoke CLI adapters, ACP evaluated per-provider, not adopted as a blanket transport yet

**Status:** Accepted
**Date:** 2026-08-30
**Deciders:** NACC Phase 0 foundation audit

## Context

The master plan's §9.4 treats ACP (Agent Client Protocol) as a Copilot-only,
version-gated *extra* on top of bespoke per-CLI adapters. A docs-only
research pass during this session initially suggested ACP might deserve a
bigger role — reporting that Claude, Codex, Copilot, Gemini, and OpenCode
were all "listed as ACP agents" on `agentclientprotocol.com`, which would
argue for building `nacc-provider-core` around ACP as the primary transport
with bespoke adapters as fallback.

That third-party listing turned out to be a poor guide to what the actually
installed CLIs on this machine support. Live testing corrected it
substantially.

## Evidence

Live-probed every installed CLI's own `--help` output for an ACP-labeled
flag or subcommand, then attempted a real ACP `initialize` handshake against
the two that had one.

| Provider | Native ACP flag? | Live handshake result |
|---|---|---|
| **Copilot** 1.0.82 | **Yes** — `--acp`, *"Start as Agent Client Protocol server"* | Real server process confirmed (full startup sequence in `~/.copilot/logs/`). Blocked before `initialize` response by Copilot's own auth requirement — `No authentication information found`, and a classic PAT (`gh`'s token) is explicitly rejected: *"Classic Personal Access Tokens (ghp_) are not supported by Copilot."* Deliberately not bypassed by creating a new credential. |
| **Gemini CLI** 0.40.0 | **Yes** — `--acp` (+ deprecated `--experimental-acp`), *"Starts the agent in ACP mode"* | Real process, immediate failure at its own eligibility gate: `IneligibleTierError` — Google has sunset individual free-tier Gemini Code Assist access and redirects to Antigravity. Not an ACP protocol failure; an account-tier failure. |
| **Claude Code** 2.1.215 | **No** — zero matches for `acp`/`agent-client`/`agent.client` in full `--help` text | Not applicable. A separate, real, currently-published bridge package exists (`@zed-industries/claude-code-acp@0.16.2`, also `claude-code-acp`/`cc-acp@0.1.1`) that wraps the CLI to speak ACP. Confirmed via `npm view` against the live registry. |
| **Codex** 0.149.1 | **No** — zero matches across `codex --help`, `codex exec --help`, `codex app-server --help` | Not applicable. `app-server` is a real, shipping, but entirely separate protocol (own stdio/unix-socket/websocket transports, own websocket auth modes) — not ACP-framed. |
| **Antigravity** | Unknown — no local `agy` binary to test | Not testable this session. |
| **OpenCode** | Unknown — headless CLI is currently broken on this machine (missing binary) | Not testable this session. |

A structural pattern held across both live handshake attempts: **each
provider's own auth/eligibility gate ran before the ACP JSON-RPC layer ever
engaged.** Neither failure was protocol-level. This means an ACP contract
test can never be "spawn process, expect JSON-RPC response" in the
abstract — it always needs a real, currently-valid, mode-appropriate
credential, and "no such credential available" needs to be its own
first-class, testable failure state distinct from a timeout or malformed
output.

## Decision

**Do not adopt ACP as `nacc-provider-core`'s primary or default transport.**
Keep bespoke per-CLI adapters as the baseline contract for every provider
(as the master plan already specifies), and treat ACP as an **optional,
per-provider-capability-flagged alternate transport** — closer to the master
plan's original framing than to the broader bet the docs-only pass
suggested, but broader than "Copilot only":

1. **Copilot**: ACP is a real, near-term-viable optional transport once a
   fine-grained PAT or `copilot login` session is available. Verify the full
   `initialize`↔`result` round trip in Phase 4 with real credentials before
   enabling it in any capability snapshot.
2. **Gemini/Antigravity**: ACP is architecturally real (the flag exists on
   Gemini CLI) but currently blocked by Google's own tier sunset — this is
   an account/product problem, not an integration problem. Re-evaluate once
   Phase 8's Antigravity work either finds a working `agy` binary or
   confirms `codex-router`'s OAuth bridge as the actual integration point;
   ACP viability depends entirely on what surface that path exposes.
3. **Claude**: only reachable via a third-party bridge package
   (`claude-code-acp`). That is an extra dependency, extra version to pin,
   and extra trust surface compared to the CLI's own `-p
   --output-format stream-json`, which already satisfies NACC's structured-
   event requirement without it. **Do not adopt the bridge unless a concrete
   need for ACP-specific features (e.g. `session/request_permission`,
   `terminal/*` methods) emerges that the native flag surface can't cover.**
4. **Codex**: no ACP path exists. Its own `app-server` protocol is a
   separate, real capability worth evaluating on its own merits in a later
   phase, but it is not a stand-in for ACP and should not be modeled as one.
5. **OpenCode**: undetermined; blocked on getting a working binary at all
   (see `docs/provider-adapters/opencode.md`).

Concretely: `nacc-provider-core`'s `CapabilitySnapshot` (master plan §8.3)
gets one additional boolean-ish field, `acp_transport: Unsupported |
Native | Bridged | Unverified`, populated per provider from the table
above and refreshed per the plan's existing capability-refresh policy
(§2.7). The orchestrator never assumes ACP; it only uses it where a
role profile's resolved adapter reports `Native` or `Bridged` *and* the
adapter's own contract tests for that mode currently pass.

## Consequences

- No architectural bet is placed on ACP becoming the universal transport
  before there is evidence it can even complete a handshake on this user's
  actual accounts. This avoids the failure mode the docs-only pass would
  have walked into: designing the core orchestration loop around a protocol
  that, on live testing, two of five providers can't currently complete
  authentication for.
- Bespoke adapters remain the load-bearing contract for every provider, as
  originally specified. ACP becomes a genuinely useful *addition* for
  Copilot specifically once credentials allow verifying it, rather than a
  replacement architecture.
- Phase 4's adapter contract test suite (master plan §6) must include an
  explicit "credential available but ineligible/unauthenticated for this
  mode" test case per provider — a state this audit found live, twice, and
  which nothing in the original contract-test list (§6: installation probe,
  unauthenticated state, malformed output, timeout, cancellation, permission
  denial, rate limit, structured handoff, CLI incompatibility) names
  precisely. Recommend adding it as a first-class case rather than folding
  it into "unauthenticated state," since it is a different failure shape
  (a *valid* credential rejected for a *specific mode*, not simply no
  credential).

## Alternatives considered

- **ACP as primary transport for all providers** — rejected; two of five
  live-tested providers cannot currently complete the handshake at all on
  this user's real accounts, and two of five installed CLIs have no ACP
  surface whatsoever.
- **Ignore ACP entirely, bespoke adapters only** — rejected; Copilot's
  native `--acp` flag is real and represents a genuine near-term
  opportunity (richer, persistent sessions per master plan §9.4) once
  authenticated, and discarding it would mean re-discovering this later
  with less context than this audit already has.

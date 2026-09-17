# ADR 0001: Antigravity and OpenCode launch paths stay typed-blocked

Date: 2026-09-17. Status: accepted. Supersedes: nothing. Conflicts-with
resolution per master plan S32: this preserves the plan's intent (S9.3,
S9.5) and defers only what has no verified contract to implement against.

## Context

Phase 8 requires Antigravity and OpenCode execution. Phase 0's audit
(docs/provider-adapters/) found: no headless `agy` CLI exists on the
audited machine, and the installed OpenCode CLI is broken (wrapper points
at a never-installed binary). The build prompt's constraints forbid
pretending a setting or contract works, and forbid claiming support where
evidence is absent.

## Decision

- Everything with a grounded contract is implemented and fixture-tested:
  Antigravity's S9.3 structured handoff schema and evidence cross-checks
  (`nacc-provider-antigravity::handoff`), OpenCode's S9.5 gateway profile
  and model discovery (`nacc-provider-opencode::{profile,models}`).
- `launch`/`resume` for both adapters keep returning the typed
  `ProviderError::Other("... not implemented yet ...")` from the Phase 1
  stubs. No argv, flag, or output format has been invented.
- Registering either adapter in the app registry is deferred until a real
  CLI can be probed; `build_registry` still registers exactly Claude and
  Codex, and its test pins that.

## Consequences

When a real `agy` or working `opencode` binary is available, the work is:
capture fixtures, implement launch parsing behind the existing
`CommandRunner` seam exactly as the Claude/Codex adapters did, add the
contract suite, then register. The handoff schema and profile surfaces
here are the contract those adapters must satisfy.

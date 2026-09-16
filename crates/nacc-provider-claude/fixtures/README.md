# Claude Code adapter fixtures

## Provenance, stated per file

Fixtures exist so this adapter's real code path (argv construction, output
interpretation, error classification, session lifecycle) runs in CI on a
machine with no `claude` binary installed. What each file is *actually
based on* matters, so it is recorded here rather than implied.

| File | Provenance |
|---|---|
| `invocations.json` | `--version` output captured from the installed binary documented in `docs/provider-adapters/claude-code.md` (`2.1.215 (Claude Code)`). |
| `stream-json-run.json` | **Documented-shape sample, not captured output.** Phase 0 never ran a live `--output-format stream-json` session (the contract doc says so explicitly under "What was NOT verified this session"). Each line follows the documented event shape (`system`/`assistant`/`user`/`result`) and is labelled here as a shape fixture. |

## What that means for confidence

- The argv the adapter builds is asserted directly (`observed_commands()`),
  so a flag change is caught regardless of fixture provenance.
- Output interpretation is exercised against documented shapes. Running the
  adapter against a real CLI with `--output-format stream-json` is the step
  that would convert `stream-json-run.json` into a captured fixture; until
  then, no test in this crate claims the live stream shape was verified.
- The interpreter is written to tolerate that: an unrecognized event type is
  logged at TRACE and ignored, rather than being forced into a normalized
  event whose meaning it might not have.

## Argument wildcards

`"args": ["--session-id", "*"]` is an argument pattern: `*` matches exactly
one argument of any value. NACC assigns a fresh session UUID per run, so a
fixture that could not express that would have to stop covering the launch
command line at all.

# Provider adapter contract — Google Antigravity

**Status:** No headless CLI is installed on this machine. This document
records what is actually present, what the docs-only research pass found
about a hypothetical `agy` CLI, and — most importantly — a live finding that
changes the priority of this adapter for the user's stated goals.

## What is actually installed

| | |
|---|---|
| Antigravity IDE | **Installed.** `Antigravity.exe`, `C:\Users\Dr Faisal Maqsood PC\AppData\Local\Programs\Antigravity\`. Electron/VSCode-fork (confirmed via `LICENSE.electron.txt`, Chromium `.pak` files, `CachedExtensionVSIXs`). Cache directories modified today — in active use. |
| Config directory | `~/.antigravity` — present, contains `argv.json` and an `extensions` dir. |
| `resources/bin/` | `language_server.exe`, `webm_encoder.exe` only — **no CLI shim**. |
| `Antigravity.exe --version` | Returns `v24.14.0` — this is the bundled Electron/Node runtime version, not a product version. **Not usable as evidence of anything.** |
| Standalone `agy` headless CLI | **Not found anywhere on this machine.** Searched PATH, `~/.local/bin`, `AppData\Local\Programs`, and a broad filesystem sweep. Genuinely absent. |

**Correction to an earlier finding in this session**: a first-pass probe
concluded "Antigravity: NOT installed" because it only checked
`Get-Command antigravity` against PATH. That check was correct as far as it
went (no CLI on PATH) but incomplete — the IDE application itself is
installed and actively used. Both statements are true and not in tension:
*the product* is installed; *the headless CLI the master plan's adapter
design assumes* is not.

## What the docs-only pass found (unverified locally — no `agy` to test)

From `antigravity.google/docs/cli/*`, not independently re-verified this
session since there is no local binary to test against:

- Binary name `agy`; documented Windows install via
  `irm https://antigravity.google/cli/install.ps1 | iex`.
- Headless mode: `-p`/`--print`/`--prompt`, `--output-format
  text|json|stream-json`, `--model <slug>`, **`--effort low|medium|high`**
  (a first-class flag, only three levels — narrower than the canonical
  scale), `--continue`/`-c`, `--conversation <id>`, `--dangerously-skip-permissions`.
- No published version number and no public GitHub repository were found —
  a real supply-chain/reproducibility concern distinct from whether it works:
  there is no way to pin an exact `agy` version the way `AgentPanel`'s SHA or
  Tauri's crates.io version can be pinned.

Treat every bullet above as **UNVERIFIED** until a real `agy` binary can be
tested. This document exists specifically so that gap is visible rather than
silently assumed away.

## Live finding that changes this adapter's priority

Testing Gemini CLI's ACP support (same session, see `gemini.md`) surfaced an
unambiguous, first-party error:

```
Error authenticating: IneligibleTierError: This client is no longer supported
for Gemini Code Assist for individuals. To continue using Gemini, please
migrate to the Antigravity suite of products: https://antigravity.google
  tierId: 'free-tier', reasonCode: 'UNSUPPORTED_CLIENT'
```

Read together with `~/.gemini/antigravity-cli/settings.json` being the
documented settings path for the (unverified) `agy` CLI, the picture is
consistent: **Antigravity is the direct successor to Gemini CLI**, and
Google's own client is actively refusing individual-tier Gemini CLI access
in favor of it. This is not just "Antigravity is a nice-to-have option" —
for an individual/Pro-tier Google account, **it may be the only currently
supported path to that quota at all.**

That raises the real stakes on the "no headless CLI installed" finding
above. It also directly informs the user's explicit request to prefer
Antigravity/Google AI Pro quota by default across roles (recorded in the
plan addendum): the practical blocker isn't the architecture's willingness
to route to Antigravity, it's that there is currently no first-party
headless surface on this machine to route to.

## The pragmatic near-term path: `codex-router`

A legitimate, actively-maintained third-party open-source project is already
installed at `C:\Users\Dr Faisal Maqsood PC\AppData\Local\codex-router\`
(`codex-model-router`, real GitHub project — LICENSE, NOTICE.md, a Homebrew
Formula, a 92KB README, CI workflows under `.github/`). It contains dedicated
Antigravity OAuth integration modules:

```
src/antigravity-oauth-constants.mjs
src/antigravity-oauth-forwarder.mjs
src/antigravity-oauth-onboarding.mjs
src/antigravity-oauth-session.mjs
src/antigravity-oauth-shape.mjs
src/antigravity-oauth-status.mjs
src/antigravity-project.mjs
test/antigravity-cli-onboarding.test.mjs
test/antigravity-oauth-forwarder.test.mjs
test/antigravity-provider-lifecycle.test.mjs
```

This was **not audited further in Phase 0** — it is real, substantial
third-party middleware whose contract needs its own dedicated review, and
deep-diving it here would have exceeded this phase's scope (see the plan's
"Provider CLI contracts are derived from official documentation... not from
[existing local tooling]" directive, which applies with equal force to this
project). It is recorded here as the **first thing to evaluate in Phase 8**
(Antigravity + OpenCode), ahead of waiting for an official headless `agy`
CLI to materialize — since it already exists, is real, and appears to solve
exactly the OAuth-bridging problem this adapter needs solved.

## What NACC must not do

Per the master plan's honest-degraded-mode principle (§2.7, §17): until a
real headless interface is verified — whether that is an official `agy`
CLI, `codex-router`'s bridge, or something else — the Antigravity provider
row in the Role Matrix must show as **unavailable/not yet supported**, not
as a working option with silently-faked capabilities. Do not build an
adapter against the docs-only `agy` contract above without testing it
against a real binary first.

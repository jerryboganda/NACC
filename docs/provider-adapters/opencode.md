# Provider adapter contract — OpenCode

**Status:** Verified live that OpenCode's CLI is currently non-functional on
this machine, tracing the failure to a specific missing file — genuinely
useful negative evidence, not a probe methodology gap.

## What is installed, and what is broken

Multiple shims exist but none resolve to a working binary:

| Path | What it is |
|---|---|
| `~/bin/opencode.cmd` | Thin batch shim → `opencode_wrapper.ps1` |
| `~/bin/opencode_wrapper.ps1` | A substantial (5.1 KB) custom PowerShell wrapper with project-specific auto-activation blocks for unrelated projects, plus HTTP-based server discovery logic (`Get-OpenCodeListener`, `Start-HeadlessOpenCodeServer`) |
| `$exe` in the wrapper | Resolves to `%LOCALAPPDATA%\OpenCode\opencode-cli.exe` |
| That path | **Does not exist.** `AppData\Local\OpenCode\` contains only a `.sisyphus` subdirectory — no `.exe` anywhere. |

Confirmed `opencode --version` (via the wrapper) exits 0 with **completely
empty output** — no error, no version string. This is why an earlier probe
reported "installed; `--version` silent" without further explanation: the
wrapper's `$exe` variable points at a binary that was never actually placed
at that path, and the wrapper apparently swallows the resulting failure
rather than surfacing it.

Also checked: a Scoop bucket manifest for OpenCode **exists**
(`~/scoop/buckets/main/bucket/opencode.json`, pointing at
`github.com/anomalyco/opencode` releases, current manifest version `1.18.15`)
but OpenCode was **never actually installed via Scoop** — `~/scoop/apps/`
has no OpenCode entry. A separate **OpenCode Desktop** app is present
(`AppData\Local\Programs\@opencode-aidesktop`, `AppData\Roaming\ai.opencode.desktop`
with real workspace state files, actively used across multiple projects
including this machine's `D:\Projects\` tree) — that is a GUI application,
not the headless CLI this adapter needs, and was not investigated further.

**Bottom line: OpenCode's headless CLI is genuinely absent/broken on this
machine as of this session**, despite look-alike shims suggesting otherwise.
Any adapter work in Phase 8 needs a clean reinstall first
(`scoop install opencode`, or fetch the release ZIP directly per the bucket
manifest) before any of the flag surface below can be verified live.

## What the docs-only pass found (unverified locally)

From `opencode.ai/docs/*` and the GitHub release history, not independently
re-verified this session:

- Latest release `v1.18.25` (2026-08-28); npm `opencode-ai@1.18.25`.
- `opencode run [message..]`, `--format json|default`, `--model <provider/model>`,
  `--continue`/`-c`, `--session`/`-s <id>`, `--agent`.
- `opencode serve` — real standout feature if it works: defaults to
  `127.0.0.1:4096`, publishes a full OpenAPI 3.1 spec at `/doc`, SSE event
  streams at `/event` and `/global/event`, HTTP basic auth via
  `OPENCODE_SERVER_PASSWORD`. This is the mechanism the local
  `opencode_wrapper.ps1` is actually built around (`Start-HeadlessOpenCodeServer`
  spawns `opencode serve --hostname 127.0.0.1 --port 0` and polls for a
  listening port) — so the *design* on this machine already assumes exactly
  this server-mode contract; it just can't run because the binary is
  missing.
- Custom provider / BYOK support via `opencode.json`
  (`$schema: https://opencode.ai/config.json`), `options.baseURL`,
  `options.apiKey`.
- Reasoning-effort flag: **not found** in the docs pass. Cancellation
  semantics for `opencode run`: **not found**.

Treat all of the above as UNVERIFIED until a working binary is available to
test against.

## What NACC must not do

Do not build the OpenCode adapter against the docs-only contract above
without first getting a real, working `opencode` binary on a test machine
and confirming the flag surface live — the same principle applied to every
other adapter in this document set. The existing local wrapper's design
(poll for a locally-served OpenAPI/SSE endpoint) is a reasonable blueprint
for `nacc-provider-opencode`'s launch/health-check logic once a real binary
is available to verify it against.

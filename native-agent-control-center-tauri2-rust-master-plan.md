# Native Agent Control Center (NACC)
## Tauri 2 + Rust Master Architecture and Implementation Plan

**Document status:** Implementation-ready master plan  
**Target platform:** Windows 10/11, Windows-first with later Linux/macOS portability  
**Desktop framework:** Latest stable Tauri 2 release at implementation time  
**Privileged/application backend:** Rust  
**Frontend:** React + TypeScript + Vite inside the Tauri webview  
**Primary execution model:** Multiple native coding-agent CLIs, each retaining its own authentication, tools, model catalog, and runtime behavior  
**Working name:** Native Agent Control Center (NACC)

---

# 1. Executive decision

Build a dedicated, local-first Windows desktop application called **Native Agent Control Center (NACC)**.

NACC will be the graphical control plane above the following native agent interfaces:

- Claude Code CLI, authenticated through the user's Claude subscription.
- OpenAI Codex CLI, authenticated through the user's eligible ChatGPT workspace or a separately configured API profile.
- Google Antigravity CLI, authenticated through the user's Google account.
- GitHub Copilot CLI and, where stable enough, its Agent Client Protocol server.
- OpenCode as the common adapter for TokenRouter, B.AI, DeepSeek-family, GLM-family, Qwen-family, and other OpenAI-compatible or custom providers.
- GitHub Actions as the deterministic build, test, artifact, staging, and production deployment engine.

NACC is not itself another general-purpose LLM. It is a **role router, workflow engine, process supervisor, permission broker, worktree manager, review console, and CI/CD control center**.

The recommended starting point is a **private, pinned, security-audited fork of AgentPanel**, because it already uses the required Tauri 2 + Rust + React architecture and contains useful foundations for Windows terminals, independent Git worktrees, parallel agent sessions, Git status, GitHub pull-request/CI visibility, editor launching, session restore, installers, and updating.

This recommendation is conditional rather than blind. The first implementation milestone is a formal foundation audit. If AgentPanel fails the decision gate described later in this document, create a fresh Tauri 2 workspace and transplant only the proven PTY, worktree, GitHub, and terminal concepts. NACC must never inherit poor architecture merely to save initial time.

## 1.1 Why the previously proposed Nimbalyst foundation is rejected

Nimbalyst is Electron/TypeScript based. It may remain a UX and feature reference, but it violates the mandatory Tauri 2 and Rust-backend requirement. It must not be used as the runtime foundation.

## 1.2 Why Google Antigravity, Codex, Claude Code, Copilot, or VS Code should not be the master GUI

Each provider-native interface is valuable, but each is naturally centered on its own provider. The required system must neutrally coordinate several competing native CLIs, accounts, model catalogs, reasoning controls, permission models, worktrees, and fallbacks. A provider-owned interface is therefore a worker surface, not the neutral control plane.

VS Code should remain an optional external editor launched from NACC for any selected worktree. It should not own the orchestration state.

## 1.3 Daily user experience

The ordinary workflow should be:

1. Open NACC.
2. Select a repository.
3. Select a workflow preset, such as **Enterprise Feature**, **Fast Bug Fix**, **CI/CD Repair**, or **Read-Only Audit**.
4. Review or override the model assigned to each role.
5. Choose reasoning, thinking, permission, runtime, concurrency, fallback, and budget controls through the GUI.
6. Enter the requested coding task.
7. Click **Start Run**.
8. Watch live exploration, planning, implementation, testing, review, integration, pull-request, and CI/CD stages.
9. Intervene only where a configured approval gate requires human judgment.
10. Approve the pull request, staging promotion, or production release.

No YAML editing should be required for normal use. Every advanced setting may have an optional source-controlled representation for reproducibility, but the GUI remains the primary interface.

---

# 2. Architectural principles

NACC must follow these principles from the beginning.

## 2.1 Native CLI preservation

Do not reimplement Claude Code, Codex, Antigravity, Copilot, or OpenCode. Invoke their native executables and preserve their native authentication, model behavior, agent loops, tool support, session semantics, and provider updates.

## 2.2 Rust owns trust and orchestration

All privileged and durable behavior belongs in Rust:

- process creation and termination;
- PTY and ConPTY supervision;
- command validation;
- provider adapter logic;
- workflow state transitions;
- worktree allocation;
- Git and GitHub operations;
- policy enforcement;
- secrets access;
- persistence and migrations;
- quality-gate execution;
- audit logging;
- update verification;
- crash recovery.

The React layer presents state and sends typed commands. It must not become a hidden Node.js backend.

## 2.3 Local-first operation

NACC must run without a proprietary NACC cloud service. Provider access naturally requires the provider's network service, but orchestration metadata, plans, task records, logs, policies, presets, worktree ownership, and audit records should remain local unless the user explicitly exports or synchronizes them.

## 2.4 Deterministic verification over LLM assertion

An agent saying “tests passed” is not evidence. NACC must run and capture the actual commands and exit codes for formatting, linting, type checking, unit tests, integration tests, browser tests, builds, migration checks, security scans, and CI workflows.

## 2.5 Isolation by default

Every write-enabled agent receives a separate Git branch and worktree. Parallel writers must not share a checkout. Read-only workers may inspect a common immutable snapshot only when safe.

## 2.6 Cross-provider review

Where practical, an implementation should be reviewed by a different model family or provider. This reduces correlated mistakes and prevents a model from simply approving its own assumptions.

## 2.7 Explicit capability discovery

Do not hard-code marketing names or assume all providers expose the same controls. NACC must detect or validate:

- installed executable and version;
- authentication status;
- available models;
- structured-output support;
- streaming format;
- resume/session support;
- reasoning values;
- thinking toggle behavior;
- permission modes;
- tool allowlists and denylists;
- MCP or plugin support;
- subagent support;
- usage reporting;
- noninteractive operation;
- cancellation behavior.

A control that a provider does not support must be disabled and labeled **Unsupported** or **Managed by provider**. It must never appear to succeed while being ignored.

## 2.8 Bounded autonomy

“Full autonomy” means high autonomy inside an isolated, policy-controlled workspace. It does not mean permanent unrestricted access to the whole Windows machine, credential stores, protected branches, production servers, or destructive database commands.

## 2.9 Durable, resumable workflows

A desktop restart, provider crash, network interruption, or Windows reboot must not erase the run. The Rust workflow engine must checkpoint state and reconcile running processes and worktrees after restart.

## 2.10 Evidence-backed completion

A workflow can be marked complete only when its declared acceptance criteria and quality gates have machine-verifiable evidence.

---

# 3. High-level system architecture

```text
┌──────────────────────────────────────────────────────────────────────┐
│                    NACC — TAURI 2 WINDOWS DESKTOP                   │
│                                                                      │
│ Dashboard   Projects   Provider Registry   Model Catalog             │
│ Role Matrix Workflow Designer Live Run Center Worktree Manager       │
│ Review Center Quality Gates CI/CD Center Policies Audit Usage        │
└──────────────────────────────┬───────────────────────────────────────┘
                               │ typed Tauri IPC / channels
                               ▼
┌──────────────────────────────────────────────────────────────────────┐
│                     RUST APPLICATION CORE                            │
│                                                                      │
│ Domain Model        Durable Workflow Engine      Event Store         │
│ Provider Registry   Process/PTY Supervisor       Policy Engine       │
│ Worktree Manager    Git/GitHub Integration       Quality Gates       │
│ Secrets Broker      Quota/Concurrency Governor   Audit/Redaction     │
│ Recovery Manager    Update Manager               Diagnostics         │
└───────────────┬──────────────────┬──────────────────┬─────────────────┘
                │                  │                  │
                ▼                  ▼                  ▼
       Native Windows         WSL2 runtime       Docker/runtime
       PowerShell/ConPTY      optional agents    optional sandbox
                │                  │                  │
                └──────────────────┴──────────────────┘
                               │
         ┌─────────────────────┼────────────────────────────┐
         │                     │                            │
         ▼                     ▼                            ▼
  Claude Code CLI       OpenAI Codex CLI           Antigravity CLI
         │                     │                            │
         ├─────────────────────┼────────────────────────────┤
         ▼                     ▼                            ▼
  Copilot CLI/ACP       OpenCode adapter       TokenRouter / B.AI /
                                              DeepSeek / GLM / Qwen
                               │
                               ▼
                    Git + GitHub + GitHub Actions
                               │
                               ▼
             Pull request → CI → staging → approved production
```

---

# 4. Required technology stack

## 4.1 Desktop shell

- **Tauri 2**, latest stable release when implementation begins.
- Pin exact working versions in `Cargo.lock` and the JavaScript package lock after validation.
- Do not use floating dependency ranges for release builds.
- Use Tauri's signed updater for application updates.
- Use Tauri capabilities and scopes to minimize frontend privileges.

At the time this plan was prepared, the latest verified Tauri release was 2.11.5. This number must be rechecked against the official release channel when the implementation agent starts; “latest” must not be guessed from this document months later.

## 4.2 Rust backend

Recommended baseline:

- Stable Rust toolchain pinned through `rust-toolchain.toml` after compatibility testing.
- Tokio for asynchronous tasks and process supervision.
- Serde/serde_json for typed IPC and provider events.
- Thiserror/anyhow used deliberately: typed errors in libraries, contextual errors at application boundaries.
- Tracing + tracing-subscriber for structured local logs.
- SQLx with SQLite for durable state and migrations.
- Portable PTY or a Windows-native ConPTY layer for terminal sessions.
- Windows Job Objects for reliable process-tree cleanup.
- Notify for selected filesystem watching.
- Reqwest for HTTP integrations where a native CLI is not the better boundary.
- Zeroize/secrecy for sensitive in-memory values where appropriate.
- Keyring crate for Windows Credential Manager access.
- Tauri Stronghold may be offered as an encrypted fallback for NACC-owned secrets.

## 4.3 Frontend

- React + TypeScript + Vite.
- Strict TypeScript configuration.
- A typed IPC client generated or maintained from Rust contracts.
- TanStack Query or a similarly disciplined server-state layer for Rust-backed data.
- Zustand, Redux Toolkit, or another small predictable store only for transient UI state.
- React Flow or equivalent for the visual workflow DAG editor.
- xterm.js for interactive PTY surfaces.
- Monaco diff editor or an equivalent robust diff component.
- A production-grade component system with accessible primitives.
- Vitest and React Testing Library for frontend tests.
- Playwright for end-to-end desktop/webview flows where practical.

The frontend may use Node.js during development and bundling. It must not require a Node.js application server at runtime.

## 4.4 Storage

Use one local SQLite database per NACC user profile, with project-specific references rather than copying project contents into the database.

Suggested data groups:

- application settings;
- provider installations and profiles;
- account references and health state;
- discovered models and capability snapshots;
- role profiles;
- workflow templates and versions;
- workflow runs and node attempts;
- event stream;
- task contracts and handoffs;
- worktree allocations;
- quality-gate results;
- review findings;
- CI/CD records;
- approvals;
- policy decisions;
- usage estimates;
- audit records;
- updater and diagnostic state.

All migrations must be embedded, versioned, transactional where SQLite permits, and tested from every supported prior schema version.

## 4.5 Git strategy

Use the installed Git CLI as the default mutation engine because it respects the user's credential helpers, hooks, signing setup, filters, LFS configuration, and existing Git behavior.

Rust must invoke Git with typed argument arrays, not interpolated shell strings.

A Rust Git library may be used for read-only inspection where it is demonstrably safer or faster, but it must not create behavioral differences that surprise users.

## 4.6 GitHub strategy

Support both:

1. `gh` CLI integration for existing authenticated workflows.
2. Direct GitHub REST/GraphQL calls through a Rust client where richer or more reliable structured behavior is necessary.

The GitHub connector must expose repositories, branches, pull requests, checks, workflow runs, jobs, logs, artifacts, environments, and approval state without embedding GitHub credentials into prompts.

---

# 5. Foundation decision: AgentPanel fork with a mandatory gate

## 5.1 Why AgentPanel is the best current seed

AgentPanel already demonstrates several difficult Windows desktop foundations in the required stack:

- Tauri 2 and Rust core;
- React/TypeScript frontend;
- Windows terminal support through PTY/ConPTY-compatible mechanisms;
- separate Git worktrees per agent task;
- parallel terminal tabs;
- session persistence and restore;
- live Git status;
- GitHub pull-request and CI visibility via `gh`;
- external editor integration;
- Windows packaging and updating.

These capabilities reduce the amount of low-level work that must be reinvented.

## 5.2 Forking rules

- Fork privately into an organization-controlled repository.
- Record the exact upstream commit and license.
- Add an `upstream` remote.
- Pin the initial revision.
- Do not merge unreviewed upstream changes automatically.
- Maintain a `docs/upstream-delta.md` file.
- Keep core modifications modular and documented through architecture decision records.
- Prefer new Rust workspace crates and extension points over invasive edits to upstream terminal components.

## 5.3 Foundation audit deliverables

Before feature implementation, the agent must produce:

- `docs/audits/foundation-audit.md`;
- `docs/adr/0001-foundation-selection.md`;
- dependency and license inventory;
- Windows clean-build evidence;
- test baseline;
- security review of Tauri permissions, shell invocation, updater configuration, persistence, and IPC;
- architectural map of Rust commands, PTY handling, storage, worktrees, GitHub integration, frontend state, and installer flow;
- explicit list of modules to retain, refactor, isolate, or replace.

## 5.4 Decision gate

Continue with the fork only if all of the following are true:

- a clean Windows build is reproducible;
- the project license permits the intended private modification and distribution;
- terminal/process code can be isolated behind a safe Rust interface;
- the frontend is separable from core orchestration logic;
- migrations and durable storage can be introduced without destructive rewrites;
- worktree behavior is correct under cancellation, crashes, and Windows path edge cases;
- updater/security configuration can be hardened;
- the dependency tree is maintainable;
- the required NACC modules can be introduced without turning the application into an untestable monolith.

Create a fresh Tauri 2 workspace instead if the audit discovers critical unmaintainable coupling, unsafe arbitrary shell exposure, unrecoverable storage design, severe licensing uncertainty, or an inability to build/test the upstream revision reliably.

The greenfield fallback must still reuse proven concepts or isolated code only after license and security verification.

---

# 6. Proposed repository structure

```text
nacc/
├── Cargo.toml                         # Rust workspace
├── Cargo.lock
├── rust-toolchain.toml
├── package.json
├── pnpm-lock.yaml
├── src/                               # React/TypeScript GUI
│   ├── app/
│   ├── components/
│   ├── features/
│   │   ├── onboarding/
│   │   ├── projects/
│   │   ├── providers/
│   │   ├── models/
│   │   ├── roles/
│   │   ├── workflows/
│   │   ├── runs/
│   │   ├── terminals/
│   │   ├── worktrees/
│   │   ├── review/
│   │   ├── quality/
│   │   ├── cicd/
│   │   ├── policies/
│   │   ├── usage/
│   │   └── diagnostics/
│   ├── ipc/
│   └── test/
├── src-tauri/
│   ├── Cargo.toml
│   ├── capabilities/
│   ├── permissions/
│   ├── icons/
│   └── src/
│       ├── commands/
│       ├── state.rs
│       └── main.rs
├── crates/
│   ├── nacc-domain/                   # IDs, entities, value objects, invariants
│   ├── nacc-storage/                  # SQLite repositories and migrations
│   ├── nacc-events/                   # durable event model and subscriptions
│   ├── nacc-orchestrator/             # DAG/state machine/checkpointing
│   ├── nacc-provider-core/            # provider contracts and capabilities
│   ├── nacc-provider-claude/
│   ├── nacc-provider-codex/
│   ├── nacc-provider-antigravity/
│   ├── nacc-provider-copilot/
│   ├── nacc-provider-opencode/
│   ├── nacc-process/                  # PTY, process trees, streams, cancellation
│   ├── nacc-runtime/                  # Windows, WSL2, Docker runtime abstraction
│   ├── nacc-worktree/                 # branch/worktree lifecycle
│   ├── nacc-git/                      # typed Git operations
│   ├── nacc-github/                   # PR, Actions, checks, artifacts
│   ├── nacc-policy/                   # permissions and approvals
│   ├── nacc-quality/                  # deterministic quality gates
│   ├── nacc-review/                   # findings, diff annotations, dispositions
│   ├── nacc-secrets/                  # credential references and redaction
│   ├── nacc-observability/            # tracing, diagnostics, audit
│   └── nacc-updater/                  # signed update coordination
├── migrations/
├── schemas/
│   ├── task-contract.schema.json
│   ├── agent-handoff.schema.json
│   ├── review-result.schema.json
│   └── quality-result.schema.json
├── presets/
├── docs/
│   ├── adr/
│   ├── audits/
│   ├── architecture/
│   ├── provider-adapters/
│   ├── security/
│   ├── operations/
│   └── user-guide/
├── tests/
│   ├── fixtures/
│   ├── adapter-contracts/
│   ├── integration/
│   └── e2e/
└── .github/workflows/
```

Crates may be combined initially if justified, but dependency direction must remain clean. Provider-specific crates must depend on the provider-core contract, never on GUI components.

---

# 7. Core domain model

Use strongly typed IDs rather than raw strings across the backend.

Core entities:

- `Project`
- `RepositoryBinding`
- `RuntimeProfile`
- `ProviderInstallation`
- `ProviderAccountProfile`
- `ModelDescriptor`
- `CapabilitySnapshot`
- `RoleProfile`
- `WorkflowTemplate`
- `WorkflowTemplateVersion`
- `WorkflowNode`
- `WorkflowEdge`
- `WorkflowRun`
- `NodeRun`
- `Attempt`
- `AgentSession`
- `TaskContract`
- `AgentHandoff`
- `WorktreeLease`
- `ProcessLease`
- `QualityGateDefinition`
- `QualityGateResult`
- `ReviewFinding`
- `ApprovalRequest`
- `PolicyDecision`
- `GitHubRunReference`
- `ArtifactReference`
- `UsageObservation`
- `AuditEvent`

Important value objects:

- canonical reasoning level;
- provider-native reasoning value;
- thinking mode;
- permission profile;
- capability alias;
- model selector;
- fallback chain;
- concurrency budget;
- network policy;
- tool policy;
- protected path pattern;
- command risk classification;
- runtime location;
- retry policy;
- completion evidence;
- cost/usage budget.

---

# 8. Provider adapter architecture

## 8.1 Provider contract

Every native provider adapter must implement a common asynchronous Rust contract conceptually similar to:

```rust
#[async_trait]
pub trait AgentProvider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn display_name(&self) -> &str;

    async fn probe_installation(&self, runtime: &RuntimeProfile)
        -> Result<InstallationProbe>;

    async fn probe_authentication(&self, account: &AccountProfile)
        -> Result<AuthProbe>;

    async fn list_models(&self, account: &AccountProfile)
        -> Result<Vec<ModelDescriptor>>;

    async fn capabilities(&self, context: &CapabilityContext)
        -> Result<CapabilitySnapshot>;

    async fn validate_profile(&self, profile: &ResolvedAgentProfile)
        -> Result<ProfileValidation>;

    async fn launch(&self, request: LaunchRequest, sink: EventSink)
        -> Result<AgentSessionHandle>;

    async fn send_input(&self, session: &SessionId, input: AgentInput)
        -> Result<()>;

    async fn cancel(&self, session: &SessionId, mode: CancellationMode)
        -> Result<()>;

    async fn resume(&self, request: ResumeRequest, sink: EventSink)
        -> Result<AgentSessionHandle>;

    async fn collect_usage(&self, session: &SessionId)
        -> Result<Option<UsageObservation>>;
}
```

The exact trait may evolve, but all adapters must preserve the same responsibilities and normalized event semantics.

## 8.2 Normalized events

Map provider output to a common event vocabulary:

- session started;
- assistant text delta;
- reasoning status, without exposing private hidden reasoning content;
- tool requested;
- tool approved or denied;
- tool started;
- tool output delta;
- file changed;
- command started;
- command output;
- command completed;
- plan artifact emitted;
- handoff emitted;
- usage updated;
- approval requested;
- warning;
- recoverable error;
- terminal error;
- session completed;
- session cancelled.

Raw provider streams may be retained in redacted diagnostic logs, but workflow decisions must use normalized events and validated artifacts.

## 8.3 Capability snapshot

A capability record should include:

```text
installation_version
runtime_support
account_authentication_mode
models_discoverable
model_selection
reasoning_values
thinking_control
noninteractive_mode
structured_json_output
streaming_json_output
interactive_pty
session_resume
custom_agents
subagents
mcp
allowed_tools
excluded_tools
permission_modes
sandbox_modes
network_controls
usage_reporting
context_window_if_known
rate_limit_signals
cancellation_behavior
```

Store a timestamped snapshot and refresh it on demand, after CLI upgrades, after authentication changes, and periodically under a conservative policy.

## 8.4 Native credentials rule

NACC must not import, duplicate, inspect, or expose provider OAuth tokens unless the provider explicitly documents a safe supported API for doing so.

Instead:

- launch the provider's native login command;
- monitor its exit result;
- run a documented authentication-status command;
- store only the fact that a profile is connected, the account label if safely exposed, and a reference to the native installation/runtime.

API keys explicitly entered for TokenRouter, B.AI, or other gateways are NACC-owned secrets and must be stored in Windows Credential Manager or the approved encrypted fallback.

---

# 9. Provider-specific implementation plan

## 9.1 Claude Code adapter

Primary roles:

- orchestrator;
- architect;
- difficult root-cause analysis;
- security reviewer;
- final integration reviewer.

Required features:

- detect executable and version;
- native authentication-status check;
- launch native login when needed;
- support print/noninteractive execution;
- support JSON or streaming JSON where available;
- model selection;
- effort/reasoning selection when supported;
- permission-mode mapping;
- plan-only/read-only operation;
- allowed-tool restrictions;
- custom agent selection;
- JSON-schema constrained final artifacts where supported;
- session continuation/resume;
- clean cancellation and process-tree termination.

NACC should preserve Claude usage for high-value reasoning rather than spending it on routine file enumeration.

Suggested defaults:

- orchestrator: strongest available Claude model, maximum supported effort, plan-only permission;
- architect: strongest available Claude model, extra-high or maximum supported effort, plan-only permission;
- reviewer/security: strong Claude model, high or above, read-only permission.

Model names must be discovered or validated at runtime.

## 9.2 OpenAI Codex adapter

Primary roles:

- frontend implementation;
- backend implementation;
- refactoring;
- test repair;
- integration;
- difficult debugging.

Use the stable noninteractive Codex execution interface as the initial contract. Treat experimental app-server interfaces as optional, version-gated features until they are stable enough for production reliance.

Required features:

- `codex login` status and supported ChatGPT authentication flow;
- optional separately billed API profile;
- `codex exec` JSONL event ingestion;
- session resume;
- model selection;
- reasoning effort mapping;
- read-only, workspace-write, and danger-full-access sandbox mapping;
- approval-policy mapping;
- allowed network/workspace settings where supported;
- command and patch event normalization;
- clean cancellation.

Important billing/authentication rule:

A ChatGPT Business entitlement and an OpenAI API account are separate mechanisms. NACC must never assume that Business automatically supplies API credits. It should show the actual authenticated Codex mode and label API profiles separately.

## 9.3 Google Antigravity adapter

Primary roles:

- high-concurrency repository exploration;
- fast research;
- test generation;
- parallel implementation;
- frontend support;
- lower-cost debugging.

Required features:

- detect `agy` or the documented Antigravity CLI executable;
- support native Windows where reliable;
- support WSL2 as a first-class fallback runtime;
- launch the native browser/account authentication flow;
- list available models through the CLI;
- select a model;
- map Antigravity permission modes;
- invoke noninteractive prompt mode;
- supervise interactive mode through PTY when needed;
- support MCP/subagent settings where documented and tested.

Because structured machine output may be less complete than Claude or Codex, do not parse free-form terminal prose as authoritative completion data. Require the worker prompt to write a JSON handoff to a NACC-provided path, validate it against a schema, and cross-check claimed file and test evidence against Git and command results.

## 9.4 GitHub Copilot CLI/ACP adapter

Primary roles:

- GitHub Actions investigation;
- pull-request analysis;
- workflow-file maintenance;
- CI/CD remediation;
- GitHub-aware code review.

Two modes:

1. **Programmatic Copilot CLI mode** as the stable fallback.
2. **ACP server mode** for richer persistent sessions and custom frontend integration, only when the installed version supports the tested contract.

ACP is version-sensitive and may be preview functionality. NACC must:

- detect the version;
- run adapter contract tests;
- allow per-profile startup flags;
- launch separate ACP processes when reasoning or tool restrictions differ;
- fall back to programmatic CLI mode if the ACP contract fails.

Suggested profile examples:

```text
Copilot CI Investigator
  effort: high
  tools: repository view, shell, GitHub/gh
  permission: CI-maintainer

Copilot Reviewer
  effort: xhigh or strongest supported
  tools: view, diff, GitHub PR
  permission: read-only

Copilot Explorer
  effort: medium
  tools: view/search only
  permission: read-only
```

## 9.5 OpenCode and external gateway adapter

Use OpenCode as the first common adapter for:

- TokenRouter;
- B.AI;
- DeepSeek-family models;
- GLM-family models;
- Qwen-family models;
- other OpenAI-compatible, Anthropic-compatible, or configurable endpoints.

The account editor must support:

- provider label;
- base URL;
- protocol type;
- credential reference;
- custom headers;
- model endpoint discovery;
- manually added model IDs when discovery is unavailable;
- maximum context metadata if known;
- thinking/reasoning parameter mapping;
- request timeout;
- concurrency limit;
- rate-limit interpretation;
- pricing metadata entered by the user if the gateway does not expose it.

Do not hard-code “Quan 3.8” or assume an exact model spelling. Show exactly what the configured provider returns and allow a user-defined display alias.

A direct NACC adapter may be written later only when OpenCode cannot expose an important capability reliably.

---

# 10. Canonical GUI model controls

## 10.1 Canonical reasoning scale

Expose:

- Auto
- Off
- Minimal
- Low
- Medium
- High
- Extra High
- Maximum

Each provider adapter maps the canonical value to its supported native value.

Example behavior:

- If Codex supports `minimal`, `low`, `medium`, `high`, and `xhigh`, map directly where possible.
- If Copilot supports `low`, `medium`, `high`, `xhigh`, and `max`, map accordingly.
- If a provider exposes only a thinking toggle, show toggle semantics rather than pretending it has seven effort levels.
- If a provider manages thinking internally, display **Managed by provider**.
- If the selected model does not support the requested value, block save or request a visible fallback choice.

Never silently downgrade. A run must record requested and actual settings.

## 10.2 Thinking control

Expose:

- Auto
- On
- Off
- Managed by provider
- Unsupported

The control must be model-aware and provider-aware.

## 10.3 Model selection

A role may use either:

1. an exact provider/account/model ID; or
2. a capability alias resolved at run time.

Suggested aliases:

- `frontier_reasoning`
- `strong_architect`
- `fast_repository_explorer`
- `low_cost_researcher`
- `strong_code_implementer`
- `frontend_specialist`
- `backend_specialist`
- `test_engineer`
- `independent_reviewer`
- `security_reviewer`
- `github_ci_specialist`

Resolution must be deterministic and explainable. The GUI must show why a particular model was selected.

---

# 11. Role Matrix

The Role Matrix is the central configuration surface.

Required roles:

- Main Orchestrator
- Architect/Planner
- Repository Explorer
- External Researcher
- Frontend Implementer
- Backend Implementer
- Database/Migration Implementer
- Test Engineer
- General Reviewer
- Security Reviewer
- Accessibility/UX Reviewer
- CI/CD Investigator
- Integrator
- Release Manager

Each row must expose:

- enabled/disabled;
- provider;
- account profile;
- exact model or capability alias;
- reasoning level;
- thinking mode;
- permission profile;
- runtime: native Windows, PowerShell, WSL2, Docker, or configured remote runtime;
- working-directory strategy;
- maximum turns or provider equivalent;
- time limit;
- context budget if enforceable;
- concurrency;
- allowed tools;
- denied tools;
- network policy;
- MCP/plugin profile;
- structured-output requirement;
- fallback chain;
- retry policy;
- user budget limit;
- approval policy;
- reviewer separation rule.

## 11.1 Recommended default assignment

| Role | Preferred provider class | Permission | Typical concurrency |
|---|---|---:|---:|
| Orchestrator | strongest Claude reasoning model | plan only | 1 |
| Architect | strongest Claude reasoning model | plan only | 1 |
| Explorer | fast Antigravity or OpenCode model | read only | 3–5 |
| Researcher | TokenRouter/B.AI model via OpenCode | read + approved network | 2–4 |
| Frontend Implementer | Codex or Antigravity coding model | autonomous worktree | 1–2 |
| Backend Implementer | Codex or Claude coding model | autonomous worktree | 1–2 |
| Database Implementer | strong code model | constrained worktree | 1 |
| Test Engineer | Codex or Antigravity | autonomous worktree | 1–2 |
| Reviewer | different provider from implementer | read only | 1–2 |
| Security Reviewer | strongest reasoning model | read only | 1 |
| CI/CD Investigator | Copilot CLI/ACP | CI maintainer | 1 |
| Integrator | Claude or Codex | integration worktree | 1 |
| Release Manager | deterministic GitHub Actions + human approval | release policy | 1 |

These are presets, not hard restrictions.

---

# 12. Permission and autonomy system

## 12.1 Permission profiles

### Read Only

May inspect repository files, search code, read Git history, inspect manifests, read logs, and execute an approved non-mutating command allowlist.

Cannot edit files, install dependencies, commit, push, or access secrets.

### Plan Only

Adds permission to write planning and handoff artifacts under a designated NACC metadata directory, but not production source.

### Autonomous Worktree

May:

- edit only its assigned worktree;
- install project-local dependencies under policy;
- run declared builds and tests;
- create local commits;
- access explicitly allowed network destinations.

May not:

- modify the primary checkout;
- read unrelated credential stores;
- force-push;
- alter protected branches;
- deploy production;
- execute destructive database or infrastructure commands.

### Repository Maintainer

Adds permission to push agent branches, create draft pull requests, and update permitted workflow files.

### CI Maintainer

Adds permission to inspect and rerun allowed GitHub Actions, download logs/artifacts, and push repair branches. It does not grant unrestricted production environment control.

### Release Candidate

May merge to a configured integration branch and trigger staging after quality and approval gates.

### Temporary Danger Full Access

A per-run, time-limited override with:

- prominent warning;
- typed confirmation;
- explicit scope;
- automatic expiry;
- complete audit record;
- no ability to make itself persistent.

## 12.2 Always approval-gated operations

Regardless of role, require explicit approval for:

- force-pushing protected branches;
- deleting repositories or protected branches;
- changing repository visibility;
- changing branch protection or organization security controls;
- reading or rotating production secrets;
- destructive database migrations or data deletion;
- deleting cloud resources;
- deploying production;
- bypassing required CI checks;
- executing commands outside the declared project/runtime scope;
- enabling an unbounded shell permission profile.

---

# 13. Tauri and desktop security design

## 13.1 Minimal webview privileges

The frontend must not receive a generic shell execution capability. All execution goes through typed Rust commands with policy evaluation.

## 13.2 Tauri capabilities and scopes

- Separate capabilities by window and feature.
- Use deny rules for dangerous file, command, and URL patterns.
- Keep the main UI incapable of arbitrary filesystem or process access.
- Use a dedicated trusted window for authentication callbacks only if required.
- Never load remote arbitrary content into a privileged webview.

## 13.3 Command construction

- Use executable + argument arrays.
- Never concatenate untrusted strings into `cmd.exe /C`, PowerShell, Bash, or WSL shell commands unless a reviewed feature explicitly requires a shell.
- Normalize and validate working directories.
- Resolve symlinks/junctions when enforcing protected path boundaries.
- Treat Windows UNC paths, long paths, drive changes, and case-insensitivity as security-relevant edge cases.

## 13.4 Process containment

- Place each native Windows agent process tree in a Windows Job Object.
- Track PID, child tree, runtime, worktree lease, provider session ID, start time, and cancellation state.
- On cancellation, request graceful provider shutdown, then terminate the contained tree after a policy-controlled timeout.
- Reconcile orphaned processes after application restart.

## 13.5 Secrets

- Provider-native subscription credentials remain provider-owned.
- API keys go to Windows Credential Manager by default.
- Logs, prompts, terminal streams, crash reports, and handoff files pass through redaction.
- Environment variables are supplied by allowlist, not inherited indiscriminately.
- Protected file patterns include `.env*`, SSH keys, cloud credentials, browser profiles, Git credential stores, and user-configured paths.

## 13.6 Content Security Policy

Use a strict CSP. Avoid remote scripts, `eval`, unsafe inline script execution, and untrusted HTML rendering. Sanitize Markdown and terminal hyperlinks.

## 13.7 Signed updates

Use Tauri's signed update mechanism. The public verification key is bundled; private signing material must never be in the repository or application package. Update verification must not be bypassable through ordinary settings.

---

# 14. Workflow engine

## 14.1 Durable state machine

Suggested top-level states:

```text
Draft
→ Preflight
→ Exploring
→ Planning
→ AwaitingPlanApproval (optional by policy)
→ AllocatingWorktrees
→ Implementing
→ LocalVerification
→ Reviewing
→ Repairing (bounded)
→ Integrating
→ PullRequest
→ ContinuousIntegration
→ Staging
→ AwaitingProductionApproval
→ Deploying
→ Completed
```

Terminal states:

- Completed
- Failed
- Cancelled
- Superseded
- RequiresManualIntervention

Each node also has its own attempt state machine with leases and heartbeats.

## 14.2 DAG execution

The workflow template is a versioned directed acyclic graph.

A node declares:

- role profile reference;
- inputs;
- expected artifacts;
- dependencies;
- worktree mode;
- permission profile;
- timeout;
- retries;
- fallback behavior;
- concurrency key;
- quality gates;
- approval gates;
- completion predicate.

## 14.3 Checkpointing

Persist every state transition and important event transactionally. On restart:

1. load incomplete runs;
2. inspect process leases;
3. inspect worktree state;
4. query provider session resumability;
5. query GitHub state;
6. mark ambiguous attempts as `RequiresReconciliation`;
7. resume only after invariant checks.

## 14.4 Concurrency governor

Use semaphores keyed by provider, account, model, runtime, repository, and worktree mutation scope.

The governor should consider:

- user-configured concurrent-session limits;
- rate-limit signals;
- recent provider failures;
- daily or run budget;
- expensive-model conservation;
- repository conflict risk;
- local CPU/RAM limits;
- external service limits.

A fallback must always be visible. Record requested provider/model, actual provider/model, and reason for fallback.

## 14.5 Repair policy

Default to no more than two automated repair cycles for the same failure signature. After the bound is reached, force root-cause reassessment or manual review. Do not create endless “fix-test-fix” loops.

---

# 15. Structured contracts and handoffs

Every role receives a task contract and returns a validated handoff.

## 15.1 Task contract example

```json
{
  "schema_version": "1.0",
  "task_id": "BE-014",
  "role": "backend_implementer",
  "objective": "Implement organization-scoped RBAC checks",
  "repository": {
    "base_commit": "<sha>",
    "worktree": "<assigned path>",
    "branch": "agent/be-014"
  },
  "owned_paths": [
    "src/auth/**",
    "src/routes/organizations/**",
    "tests/auth/**"
  ],
  "forbidden_paths": [
    ".env*",
    ".github/workflows/deploy-production.yml"
  ],
  "acceptance_criteria": [
    "Cross-tenant access is denied",
    "Existing same-tenant behavior remains compatible",
    "Unit and integration tests cover allow and deny cases"
  ],
  "required_commands": [
    "npm run typecheck",
    "npm run test:unit -- auth"
  ]
}
```

## 15.2 Handoff example

```json
{
  "schema_version": "1.0",
  "task_id": "BE-014",
  "status": "completed",
  "summary": "Added organization-scoped authorization checks.",
  "files_changed": [
    "src/auth/authorization.ts",
    "src/routes/organizations.ts",
    "tests/auth/organization-rbac.test.ts"
  ],
  "commits": ["<sha>"],
  "commands_run": [
    {
      "command_id": "gate-typecheck",
      "exit_code": 0,
      "evidence_ref": "quality://..."
    }
  ],
  "risks": [
    "The migration must run before the new route is enabled."
  ],
  "review_focus": [
    "Verify cross-tenant isolation and service-account behavior."
  ]
}
```

NACC must verify `files_changed` against Git, commits against the repository, and command evidence against captured quality-gate records.

---

# 16. Worktree and integration model

Suggested layout:

```text
<repository-parent>/.nacc-worktrees/<repo-id>/
├── explorer-001/
├── frontend-014/
├── backend-015/
├── tests-016/
├── reviewer-017/       # normally read-only or detached review checkout
└── integration-018/
```

Rules:

1. Never place worktrees inside a directory recursively watched or built by the primary project unless intentionally configured.
2. Every write-enabled node owns exactly one worktree lease.
3. The primary user checkout remains untouched by default.
4. Branch names are deterministic, sanitized, and collision-safe.
5. NACC records the base commit and detects drift before integration.
6. Reviewers review diffs without writing into the implementation worktree.
7. The integrator cherry-picks or merges verified commits serially in an integration worktree.
8. Conflicts are resolved only in the integration worktree.
9. Cleanup verifies uncommitted changes, unpushed commits, running processes, and review references before deletion.
10. Abandoned worktrees are quarantined rather than forcibly removed when evidence may be lost.

---

# 17. GUI information architecture

## 17.1 First-run Setup Wizard

Steps:

1. Welcome and local security explanation.
2. Detect Git, GitHub CLI, Rust/runtime prerequisites, WSL2, Docker, VS Code, and supported agent CLIs.
3. Show exact executable paths and versions.
4. Offer provider-specific installation guidance or an explicitly approved installation action.
5. Launch native login flows.
6. Verify authentication.
7. Discover models and capabilities.
8. Run a harmless read-only smoke prompt.
9. Configure local runtime preferences.
10. Configure secrets storage.
11. Create initial role presets.
12. Validate a sample repository in read-only mode.

## 17.2 Home Dashboard

Display:

- active runs;
- waiting approvals;
- failed nodes;
- provider/account health;
- usage/rate-limit warnings;
- CI failures;
- recent projects;
- orphaned process/worktree alerts;
- application update status.

## 17.3 Projects

- add local repository;
- clone an authorized repository;
- verify Git state;
- select default branch;
- detect project technology;
- configure quality commands;
- configure protected paths;
- choose runtime;
- open primary checkout or any worktree in VS Code.

## 17.4 Providers and Accounts

Provider card fields:

```text
Provider
Installation path
CLI version
Runtime
Authentication status
Account label
Model count
Last capability refresh
Health
Concurrency limit
Native credential ownership
```

Support multiple profiles for the same provider.

## 17.5 Model Catalog

For each model:

- provider-native ID;
- display name;
- account availability;
- context information if available;
- reasoning levels;
- thinking behavior;
- tool/agent limitations;
- structured-output support;
- user-defined tags;
- cost metadata if known;
- recent success/latency observations;
- role eligibility.

## 17.6 Role Matrix

Spreadsheet-like editor with validation, compare mode, presets, bulk changes, fallback editor, and effective-setting preview.

## 17.7 Workflow Designer

Visual DAG editor with:

- drag-and-drop role nodes;
- dependencies;
- parallel branches;
- conditional edges;
- retry/fallback edges;
- approvals;
- quality gates;
- input and artifact contracts;
- per-node overrides;
- validation before save;
- version history;
- run preview.

## 17.8 Live Run Center

Show:

- workflow graph with live status;
- per-agent terminal or normalized event stream;
- current phase;
- elapsed time;
- worktree and branch;
- provider/model/reasoning actually used;
- pending tool approvals;
- files changed;
- quality evidence;
- pause/cancel controls;
- fallback and retry events;
- cost/usage observations.

## 17.9 Worktree Manager

Show lease owner, branch, base commit, changed files, commits, process state, disk usage, conflict risk, editor open button, quarantine, and safe cleanup.

## 17.10 Review Center

- tree and side-by-side diff;
- line-level comments;
- agent findings grouped by severity;
- evidence references;
- mark accepted, rejected, false positive, deferred, or fixed;
- request repair task;
- verify reviewer-provider independence;
- compare pre- and post-repair diffs.

## 17.11 Quality Gates

- configured commands;
- runtime and working directory;
- exit code;
- duration;
- full redacted logs;
- test counts;
- coverage where parsed;
- flaky-test classification;
- artifact references;
- rerun policy;
- required/optional status.

## 17.12 CI/CD Center

- pull requests;
- checks and workflows;
- jobs and steps;
- logs and annotations;
- artifacts;
- environments;
- staging status;
- release approvals;
- create a CI repair run;
- rerun only permitted jobs/workflows;
- direct link to GitHub for unsupported operations.

## 17.13 Usage and Quotas

Display only data actually exposed or user-configured. Distinguish exact provider-reported usage, locally estimated usage, subscription-session counts, API cost, and unknown values.

## 17.14 Security and Policies

- permission profiles;
- protected commands;
- protected paths;
- allowed network domains;
- environment-variable allowlists;
- deployment gates;
- branch rules;
- danger-mode history;
- secret redaction tests.

## 17.15 Audit and Diagnostics

Every important action should be queryable by run, agent, provider, project, worktree, command, approval, and user action. Export must redact secrets by default.

---

# 18. Workflow presets

## 18.1 Enterprise Feature

```text
Preflight
→ parallel repository explorers + external researcher
→ architect consolidates implementation contract
→ optional plan approval
→ parallel frontend/backend/database/test worktrees
→ deterministic local quality gates
→ independent reviewer
→ security reviewer where risk requires
→ bounded repair
→ serial integration
→ pull request
→ GitHub Actions
→ Copilot CI investigation if failed
→ staging
→ explicit production approval
```

## 18.2 Fast Bug Fix

```text
Targeted explorer
→ root-cause plan
→ one implementer worktree
→ targeted tests
→ independent review
→ pull request and CI
```

## 18.3 CI/CD Repair

```text
Copilot CI investigator
→ classify exact failing jobs/tests
→ distinguish product defect, stale test, flakiness, environment, dependency, workflow, secret, or external-service failure
→ root-cause reviewer
→ isolated repair worktree
→ affected local gates
→ pull request or repair commit
→ GitHub Actions rerun
```

Blind reruns do not count as repairs.

## 18.4 Frontend Visual Hardening

```text
UI explorer
→ implementation worker
→ responsive/browser matrix
→ Playwright visual checks
→ accessibility review
→ independent UI reviewer
→ integration and CI
```

## 18.5 Backend Security Change

```text
architecture/security exploration
→ threat notes
→ backend implementation
→ database/migration review
→ security reviewer
→ unit/integration/authorization tests
→ CI
```

## 18.6 Read-Only Audit

Parallel explorers and reviewers may inspect and report, but no source modification, commits, pushes, or workflow reruns are permitted.

---

# 19. CI/CD architecture

AI assists CI/CD; it does not replace it.

```text
NACC creates verified branch and pull request
                    ↓
              GitHub Actions
     ┌──────────────┼────────────────┐
     │              │                │
 formatting      unit tests       security scans
 lint/type       integration       build/artifacts
 browser/E2E     migrations        deployment gates
     └──────────────┼────────────────┘
                    ↓
              success or failure
                    ↓
      NACC retrieves structured check state
                    ↓
 failure → Copilot CI specialist → repair worktree
                    ↓
     local reproduction + independent review
                    ↓
       push repair → GitHub Actions rerun
                    ↓
       staging → explicit production approval
```

## 19.1 Failure classification

The CI investigator must classify failures as one or more of:

- real production defect;
- stale or incorrect test;
- test isolation failure;
- nondeterministic/flaky test;
- environment/runner failure;
- dependency or registry instability;
- workflow/configuration defect;
- build-cache corruption;
- secret/permission problem;
- external-service outage;
- timeout/resource exhaustion;
- platform-specific issue.

The classification and evidence must be visible before a repair is accepted.

## 19.2 Deployment rules

- GitHub Actions or another configured deterministic runner owns deployment.
- NACC may trigger only permitted workflows.
- Staging may be automatic after all required gates.
- Production always requires explicit approval by default.
- Deployment credentials are not exposed to agent prompts.
- Rollback instructions and artifact provenance must be recorded.

---

# 20. Quality system

## 20.1 Project quality profile

Each project defines commands for applicable gates:

- formatter check;
- lint;
- static analysis;
- type check;
- unit tests;
- integration tests;
- contract tests;
- browser tests;
- accessibility tests;
- security scans;
- dependency audits;
- migration validation;
- production build;
- packaging;
- smoke tests.

NACC may propose detected defaults but requires validation before treating them as authoritative.

## 20.2 Test isolation and flakiness

Capture repeated-run evidence and signatures. Do not hide flaky tests by automatic retries. Retries may gather evidence, but the final status must indicate that initial execution failed and whether the issue remains unresolved.

## 20.3 Adapter contract tests

Each provider adapter needs fixture-driven and live opt-in tests for:

- installation probe;
- unauthenticated state;
- model discovery;
- capability mapping;
- noninteractive run;
- streaming events;
- structured artifact;
- cancellation;
- timeout;
- resume;
- permission denial;
- rate-limit response;
- CLI upgrade incompatibility.

Live tests must never run in ordinary CI without explicit secret and cost controls.

---

# 21. Reliability and recovery

NACC must handle:

- desktop crash;
- Windows restart;
- provider CLI crash;
- frozen PTY;
- network loss;
- authentication expiry;
- rate limiting;
- disk full;
- worktree deletion outside NACC;
- branch changed externally;
- repository base branch advanced;
- Git lock file;
- partial Git operation;
- GitHub outage;
- failed updater;
- database migration interruption.

Recovery features:

- event-based durable state;
- process and worktree leases;
- startup reconciliation;
- idempotent commands;
- transaction boundaries;
- quarantined cleanup;
- resumable provider sessions where supported;
- explicit “resume from last verified checkpoint” action;
- safe export of run diagnostics.

---

# 22. Observability and audit

Use structured logs with correlation IDs:

- application session;
- project;
- workflow run;
- node run;
- attempt;
- provider session;
- process;
- worktree;
- GitHub run.

Audit events should record:

- who/what initiated an action;
- requested and actual provider/model;
- effective reasoning and permission profile;
- command executable and redacted arguments;
- working directory;
- policy decision;
- approval identity/time;
- file-change summary;
- Git commits;
- quality evidence;
- CI/CD action;
- fallback reason;
- cancellation/termination path.

Do not store hidden chain-of-thought. Store visible summaries, plans, provider status events, tools, outputs, and validated artifacts.

---

# 23. Performance requirements

- UI should remain responsive while many agents stream output.
- Use bounded channels and backpressure.
- Batch terminal rendering and database writes.
- Avoid sending enormous logs through a single Tauri command response; stream through channels/events and page persisted logs.
- Virtualize long event lists and diffs.
- Limit file watching to active worktrees and relevant paths.
- Avoid re-indexing the entire repository for every worker.
- Cache capability and model discovery with visible freshness timestamps.
- Keep expensive orchestrator context compact through structured explorer summaries.

Suggested initial local concurrency cap should be conservative and derived from CPU/RAM. Users can override it per runtime.

---

# 24. Implementation roadmap

## Phase 0 — Foundation audit and decision gate

Deliver:

- verified upstream revision;
- license and dependency audit;
- clean Windows build;
- baseline tests;
- architecture map;
- security findings;
- fork-vs-greenfield ADR.

No broad feature rewrite before this phase passes.

## Phase 1 — Tauri/Rust modular foundation

Deliver:

- current stable Tauri 2 pinned and verified;
- Rust workspace boundaries;
- typed IPC;
- tracing and error handling;
- strict capabilities/CSP;
- reproducible frontend and desktop builds;
- basic signed-updater development configuration.

## Phase 2 — Durable domain and storage

Deliver:

- SQLite schema;
- transactional migrations;
- repository layer;
- event and audit records;
- settings and profiles;
- backup/restore tests.

## Phase 3 — Process, terminal, runtime, and worktree core

Deliver:

- Windows Job Object process containment;
- ConPTY/PTY streaming;
- graceful and forced cancellation;
- WSL2 runtime abstraction;
- optional Docker runtime;
- safe Git argument invocation;
- worktree leases and recovery;
- VS Code launch integration.

## Phase 4 — Provider registry and adapter framework

Deliver:

- common provider contract;
- capability snapshots;
- model catalog;
- normalized events;
- adapter contract test harness;
- provider health UI.

## Phase 5 — Claude and Codex MVP adapters

Deliver:

- installation/authentication/model probes;
- structured noninteractive sessions;
- permission/reasoning mapping;
- cancellation/resume;
- live run UI;
- read-only explorer → planner → worktree implementer demonstration.

## Phase 6 — Setup Wizard and Role Matrix

Deliver:

- complete GUI onboarding;
- multiple accounts per provider;
- exact model and capability-alias selection;
- reasoning/thinking controls;
- fallback chains;
- validation and effective-setting preview.

## Phase 7 — Durable workflow engine

Deliver:

- versioned DAG templates;
- checkpointed state machine;
- concurrency governor;
- retries/fallbacks;
- approvals;
- pause/cancel/resume;
- crash recovery.

## Phase 8 — Antigravity and OpenCode adapters

Deliver:

- Antigravity native/WSL probing and execution;
- model and permission mapping;
- structured handoff artifact enforcement;
- OpenCode custom-provider profiles;
- TokenRouter and B.AI example profiles;
- gateway model discovery.

## Phase 9 — Review and quality system

Deliver:

- diff viewer;
- review findings;
- deterministic gates;
- structured evidence;
- bounded repair flow;
- cross-provider reviewer enforcement.

## Phase 10 — GitHub and Copilot CI/CD

Deliver:

- GitHub repository/PR/check/workflow integration;
- Copilot programmatic adapter;
- version-gated ACP adapter;
- CI failure classification;
- repair-run creation;
- staging and production approval UI.

## Phase 11 — Security hardening and reliability

Deliver:

- policy engine;
- secret storage/redaction;
- protected path/command/network rules;
- threat model;
- abuse-case tests;
- orphan reconciliation;
- migration/recovery tests;
- updater signing procedure.

## Phase 12 — Packaging, documentation, and release

Deliver:

- signed Windows installer;
- signed update channel;
- clean-machine smoke tests;
- user guide;
- administrator/security guide;
- provider troubleshooting;
- backup/restore;
- diagnostics bundle;
- release checklist.

---

# 25. Minimum viable product and expansion order

To avoid building a huge but unusable dashboard, ship capability in vertical slices.

## MVP vertical slice

- Windows Tauri app;
- one local repository;
- Claude and Codex providers;
- model/reasoning/permission selection;
- explorer → planner → implementer → reviewer workflow;
- separate worktree;
- terminal/event stream;
- deterministic quality command;
- diff review;
- local commit;
- cancellation and restart recovery.

## Second slice

- Antigravity;
- OpenCode with TokenRouter/B.AI;
- role presets;
- parallel workers;
- fallback chain;
- richer quality gates.

## Third slice

- GitHub pull requests and Actions;
- Copilot CI specialist;
- CI repair flow;
- staging and release approvals.

## Fourth slice

- visual workflow designer;
- quota governor;
- enterprise policy packs;
- advanced reporting and extensibility.

---

# 26. Non-functional requirements

## Security

- least privilege;
- no plaintext secrets;
- no general shell from webview;
- signed updates;
- policy and audit coverage;
- no production deployment without approval.

## Reliability

- durable runs;
- idempotent recovery;
- bounded retries;
- process-tree cleanup;
- migration safety;
- diagnostic export.

## Performance

- responsive live UI;
- bounded streams;
- log pagination;
- limited watchers;
- efficient SQLite writes;
- no unnecessary model context duplication.

## Maintainability

- modular Rust crates;
- provider contracts;
- ADRs;
- adapter fixtures;
- strict lint and test gates;
- documented upstream delta.

## Accessibility

- keyboard navigation;
- screen-reader labels;
- visible focus;
- sufficient contrast;
- scalable text;
- reduced-motion support;
- terminal and diff alternatives where practical.

---

# 27. Acceptance criteria

NACC is not complete until the following are demonstrated with evidence:

1. It is a Tauri 2 desktop application, not Electron.
2. Privileged application and orchestration behavior is implemented in Rust.
3. No Node.js server is required at runtime.
4. It installs and launches on a clean supported Windows machine.
5. The first-run wizard detects supported CLIs and exact versions.
6. Native authentication can be initiated and verified without exposing provider credentials.
7. Claude Code, Codex, Antigravity, Copilot, and OpenCode profiles can be represented through the GUI.
8. At least Claude and Codex are fully operational in the first production milestone.
9. Available models are discovered or explicitly validated rather than assumed.
10. Every role can select provider, account, model/alias, reasoning, thinking, permission, runtime, concurrency, and fallback.
11. Unsupported controls are disabled and clearly explained.
12. Requested and actual model/reasoning settings are audited.
13. Write-enabled agents use independent Git worktrees.
14. The primary checkout remains unmodified during an ordinary autonomous run.
15. Parallel writers cannot acquire the same worktree lease.
16. Windows process trees are terminated reliably on cancellation.
17. A run survives application restart and reconciles its state safely.
18. Structured handoffs are schema-validated and cross-checked with Git and command evidence.
19. Deterministic quality gates, not model claims, determine success.
20. Review can be assigned to a different provider family from implementation.
21. Repair loops are bounded.
22. Diff review and findings are visible in the GUI.
23. GitHub pull-request and Actions state are visible in the GUI.
24. A failed CI run can create a dedicated repair workflow and worktree.
25. CI failures are classified with evidence rather than merely rerun.
26. Production deployment is approval-gated.
27. Secrets are stored through Windows Credential Manager or an approved encrypted fallback.
28. Logs, prompts, crash reports, and exports redact configured secrets.
29. The webview cannot invoke an unrestricted shell.
30. Tauri capabilities and CSP pass a security review.
31. The updater verifies signed packages.
32. Provider adapter contract tests detect incompatible CLI upgrades.
33. SQLite migrations are reversible where feasible and upgrade-tested.
34. The application exposes pause, cancel, resume, quarantine, and safe cleanup controls.
35. The user can open any worktree in VS Code.
36. Usage values distinguish exact, estimated, and unavailable data.
37. No proprietary NACC cloud service is required.
38. Complete installation, security, recovery, provider, and user documentation is included.
39. Clean-machine smoke tests pass for the signed Windows package.
40. A complete demonstration proves explorer → planner → parallel implementation → test → independent review → integration → PR → CI flow.

---

# 28. Explicit exclusions for the first release

Unless separately approved, do not make the first release depend on:

- a hosted NACC backend;
- mobile applications;
- collaborative multi-user cloud editing;
- direct Kubernetes or cloud-console administration;
- unrestricted production automation;
- a custom LLM inference server;
- replacement of GitHub Actions;
- reimplementation of native provider authentication;
- support for every possible coding-agent CLI before the core workflow is reliable.

---

# 29. Principal risks and mitigations

## Provider CLI instability

**Risk:** flags, output formats, model names, or auth flows change.  
**Mitigation:** capability probes, version gates, adapter contracts, fixtures, visible health, and fallback paths.

## Preview protocol dependence

**Risk:** Copilot ACP or another protocol changes.  
**Mitigation:** treat it as optional and keep programmatic CLI fallback.

## Terminal prose ambiguity

**Risk:** a provider lacks reliable structured output.  
**Mitigation:** require a JSON handoff file and independently verify claims.

## Parallel editing conflicts

**Risk:** workers overwrite each other or integrate stale code.  
**Mitigation:** worktree leases, file ownership, base-commit checks, serial integration, conflict review.

## Excessive permissions

**Risk:** autonomous agents damage the machine or infrastructure.  
**Mitigation:** typed Rust policy enforcement, worktree scopes, Job Objects, protected operations, secrets isolation, explicit approvals.

## Subscription assumptions

**Risk:** a UI assumes a plan includes a model or API quota that it does not.  
**Mitigation:** inspect actual CLI auth/model availability and keep subscription and API profiles separate.

## Fork maintenance

**Risk:** upstream AgentPanel changes become difficult to merge.  
**Mitigation:** pin revision, isolate NACC crates, maintain upstream delta, cherry-pick selectively, retain greenfield escape hatch.

## GUI complexity

**Risk:** every possible option makes the product difficult to use.  
**Mitigation:** simple presets first, advanced controls behind expandable panels, effective-setting preview, safe defaults.

---

# 30. Final recommendation

Proceed with **Option A: multiple native CLIs**, managed by a purpose-built **Tauri 2 + Rust Windows GUI**.

Use AgentPanel only as a security-audited seed for terminal, worktree, GitHub, and Windows desktop foundations. Build the orchestration, provider capability system, policy engine, durable state machine, role matrix, model router, review system, and CI/CD coordination as modular Rust-owned NACC capabilities.

This gives the desired flexibility without forcing daily command-line management:

```text
One GUI
  → many native authenticated CLIs
  → role-specific models and reasoning controls
  → isolated parallel worktrees
  → deterministic tests and review
  → GitHub Actions for CI/CD
  → visible approvals and evidence
```

---

# 31. Primary implementation references

The implementation agent should recheck current versions and contracts directly from official sources before coding:

- Tauri architecture and Tauri 2 documentation: https://v2.tauri.app/
- Tauri releases: https://github.com/tauri-apps/tauri/releases
- Tauri shell plugin security/scopes: https://v2.tauri.app/plugin/shell/
- Tauri SQL plugin: https://v2.tauri.app/plugin/sql/
- Tauri updater: https://v2.tauri.app/plugin/updater/
- Tauri Stronghold: https://v2.tauri.app/plugin/stronghold/
- AgentPanel repository: https://github.com/GrillerGeek/AgentPanel
- Claude Code CLI reference: https://code.claude.com/docs/en/cli-reference
- Claude Code programmatic/headless use: https://code.claude.com/docs/en/headless
- OpenAI Codex CLI documentation: https://developers.openai.com/codex/cli/
- OpenAI Codex authentication: https://developers.openai.com/codex/auth/
- OpenAI Codex configuration: https://developers.openai.com/codex/config-reference/
- Google Antigravity CLI codelab/documentation: https://codelabs.developers.google.com/genai-for-dev-antigravity-cli
- GitHub Copilot CLI: https://docs.github.com/en/copilot/concepts/agents/copilot-cli/about-copilot-cli
- GitHub Copilot ACP server: https://docs.github.com/en/copilot/reference/copilot-cli-reference/acp-server
- OpenCode provider configuration: https://opencode.ai/docs/providers/
- TokenRouter documentation: https://docs.tokenrouter.io/
- B.AI documentation: https://docs.b.ai/

---

# 32. Handoff to the implementing agent

Use the companion file **`native-agent-control-center-tauri2-rust-build-prompt.md`** as the direct execution prompt. This master plan is the governing product and architecture specification. Where the implementation agent discovers a conflict with a current provider contract or security limitation, it must document the evidence in an ADR, preserve the intent of this plan, and choose the safest maintainable implementation rather than silently weakening a requirement.

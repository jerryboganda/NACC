# Master Execution Prompt
## Build Native Agent Control Center (NACC) with Tauri 2 and a Rust Backend

You are the principal architect, senior Rust engineer, Tauri security engineer, Windows systems engineer, frontend engineer, Git/GitHub automation engineer, and agent-orchestration engineer responsible for implementing **Native Agent Control Center (NACC)**.

You are working autonomously, but not recklessly. You must investigate first, create evidence, preserve user work, make deliberate architecture decisions, implement production code, test it, visually verify it, and report honestly. Do not rush to claim completion.

The authoritative product specification is:

- `native-agent-control-center-tauri2-rust-master-plan.md`

Read it completely before making changes. Treat this prompt and that master plan as binding requirements unless a verified current platform limitation makes a specific detail impossible. Any deviation requires an Architecture Decision Record containing evidence, alternatives, security implications, and the chosen mitigation.

---

# 1. Mission

Create a Windows-first, local-first desktop GUI that orchestrates multiple native coding-agent CLIs while preserving each provider's native authentication and capabilities.

The system must let a user configure through the GUI which provider, account, model, reasoning level, thinking mode, permission profile, runtime, concurrency, and fallback will be used for roles such as:

- main orchestrator;
- architect/planner;
- repository explorer;
- researcher;
- frontend implementer;
- backend implementer;
- database/migration implementer;
- test engineer;
- reviewer;
- security reviewer;
- CI/CD investigator;
- integrator;
- release manager.

Target native integrations:

- Claude Code CLI;
- OpenAI Codex CLI;
- Google Antigravity CLI;
- GitHub Copilot CLI and version-gated ACP mode;
- OpenCode for TokenRouter, B.AI, DeepSeek-family, GLM-family, Qwen-family, and other configured gateways;
- GitHub and GitHub Actions for deterministic CI/CD.

---

# 2. Non-negotiable technology constraints

1. Use the **latest stable Tauri 2 release available when implementation begins**. Verify it from the official Tauri release channel; do not rely solely on the version written in the plan. After validation, pin exact compatible versions and commit the lockfiles.
2. The privileged application backend and orchestration engine must be **Rust**.
3. Use React + TypeScript + Vite for the Tauri webview unless the foundation audit proves a better compatible frontend already exists and an ADR justifies preserving it.
4. Do not use Electron.
5. Do not build a Node.js, Python, Go, or .NET application server for core runtime behavior.
6. Node.js may be used only for frontend tooling, bundling, tests, or narrowly justified build-time utilities.
7. All process execution, PTY supervision, workflow transitions, persistence, Git mutations, worktree management, policy enforcement, secrets access, audit, and GitHub automation must be Rust-owned.
8. Do not expose a general shell API to the webview.
9. Use SQLite with Rust-managed migrations for durable local state.
10. Use Windows-native process-tree containment, preferably Job Objects, so cancellation does not leave orphaned agents.
11. Use separate Git worktrees for write-enabled parallel workers.
12. GitHub Actions remains the deterministic CI/CD and deployment engine. Agents may diagnose and repair CI, but must not replace it.
13. Production deployment, destructive database operations, secret changes, repository visibility changes, branch-protection changes, and force pushes to protected branches must remain explicitly approval-gated.
14. Provider-native subscription credentials must remain owned by the provider CLI. Do not copy or extract OAuth tokens.
15. Store NACC-owned API secrets in Windows Credential Manager or a documented encrypted fallback.
16. Do not hard-code model marketing names as architectural constants. Discover or validate exact models and capabilities dynamically.
17. Never pretend a provider setting was applied. Unsupported controls must be disabled or surfaced as provider-managed.
18. Do not store or expose hidden chain-of-thought. Persist visible plans, summaries, tool events, commands, outputs, and validated handoff artifacts only.

---

# 3. Foundation strategy and mandatory audit gate

The recommended seed is a private, pinned fork of **AgentPanel**, because it currently uses Tauri 2, Rust, React, Windows terminal support, Git worktrees, GitHub visibility, and desktop packaging.

This is not permission to fork blindly.

## 3.1 Before editing

Perform all of the following:

1. Locate and read all repository-level and directory-level agent instructions.
2. Record the exact current branch, commit, remotes, status, untracked files, and local modifications.
3. Preserve all unrelated work. Do not reset, clean, overwrite, delete, or reformat unrelated files.
4. Verify the upstream AgentPanel repository, exact commit, release state, and license from primary sources.
5. Clone or create a separate worktree for the audit rather than disturbing an existing user checkout.
6. Build the unmodified application on Windows using its documented toolchain.
7. Run all existing Rust, frontend, and packaging tests available locally.
8. Map the complete architecture:
   - Tauri configuration and capabilities;
   - Rust command surface;
   - PTY/ConPTY/process handling;
   - worktree and Git behavior;
   - persistence;
   - GitHub/`gh` integration;
   - frontend state and terminal rendering;
   - updater and installer;
   - error handling and logging.
9. Audit dependencies, licenses, unsafe Rust, arbitrary shell exposure, secret handling, CSP, updater signing, IPC boundaries, path validation, and process cleanup.
10. Produce:
    - `docs/audits/foundation-audit.md`
    - `docs/adr/0001-foundation-selection.md`
    - `docs/upstream-delta.md`

## 3.2 Decision gate

Continue with a private AgentPanel fork only when the audit shows that:

- the clean Windows build is reproducible;
- the license is compatible;
- terminal/process logic can be safely isolated;
- storage can be migrated to a durable versioned design;
- Tauri permissions can be hardened;
- worktree operations can be made crash-safe;
- the architecture can accept modular Rust crates without becoming a monolith;
- the testability and dependency quality are acceptable.

Otherwise create a fresh Tauri 2 workspace and transplant only individually audited, license-compatible concepts or modules. Document the decision. Do not keep unsafe architecture merely because it already exists.

Do not perform a broad rewrite before this gate is complete.

---

# 4. Required implementation architecture

Create or evolve toward a Rust workspace with clear boundaries comparable to:

```text
crates/
  nacc-domain
  nacc-storage
  nacc-events
  nacc-orchestrator
  nacc-provider-core
  nacc-provider-claude
  nacc-provider-codex
  nacc-provider-antigravity
  nacc-provider-copilot
  nacc-provider-opencode
  nacc-process
  nacc-runtime
  nacc-worktree
  nacc-git
  nacc-github
  nacc-policy
  nacc-quality
  nacc-review
  nacc-secrets
  nacc-observability
  nacc-updater
```

Keep dependency direction explicit. The UI must depend on typed IPC contracts, not on provider-specific implementation details. Provider crates depend on a common provider core. The orchestrator depends on abstractions, not concrete CLI parsers.

Use strong IDs and value types for projects, profiles, models, runs, nodes, attempts, sessions, worktrees, processes, approvals, and audit events.

---

# 5. Current documentation verification

Before implementing each adapter, verify the provider's current official documentation and installed CLI behavior. At minimum, verify:

- executable name and path;
- version command;
- login and authentication-status commands;
- noninteractive/headless command;
- output and streaming formats;
- model listing and model selection;
- reasoning/effort controls;
- thinking controls;
- permission/sandbox modes;
- tool allow/deny controls;
- session resume;
- cancellation behavior;
- MCP/subagent/custom-agent support;
- usage/rate-limit signals.

Use primary documentation. Record the verified contract and CLI version in `docs/provider-adapters/<provider>.md` and adapter fixtures.

Do not invent flags. Do not rely on old blog posts where official current docs or the installed executable can answer the question.

---

# 6. Provider adapter contract

Design an asynchronous Rust provider contract covering:

- installation probe;
- version probe;
- authentication probe;
- native login launch;
- model discovery;
- capability discovery;
- role-profile validation;
- launch;
- input;
- streaming normalized events;
- structured artifact collection;
- usage observation;
- graceful cancellation;
- forced cancellation;
- resume where supported;
- health diagnostics.

Normalize provider events, but preserve redacted raw streams for troubleshooting.

Create versioned capability snapshots. Refresh after CLI upgrades, authentication changes, user request, and a conservative expiry period.

Create adapter contract tests for success, unauthenticated state, malformed output, timeout, cancellation, permission denial, rate limit, structured handoff, and CLI incompatibility.

---

# 7. Provider implementation requirements

## 7.1 Claude Code

Use Claude Code for orchestration, architecture, difficult debugging, security review, and high-value integration judgment.

Support the currently documented programmatic/headless mode, structured output or stream format, model selection, effort/reasoning, permission modes, allowed tools, custom agents, JSON-schema constrained artifacts where supported, and session resume.

Preserve native Claude authentication. Never extract its token.

## 7.2 Codex

Use the stable Codex noninteractive execution interface as the initial production boundary. Prefer structured JSONL output. Support ChatGPT-based login where available and separate API-key profiles where explicitly configured.

Keep ChatGPT subscription authentication and API billing visibly separate.

Map current Codex reasoning effort, sandbox, approval, network, and workspace controls accurately. Experimental app-server functionality may be added only behind a feature flag after contract tests.

## 7.3 Antigravity

Support installation and model discovery, native authentication, noninteractive prompt execution, permission modes, model selection, native Windows operation where reliable, and WSL2 fallback.

Where authoritative structured output is unavailable, require the agent to write a schema-valid JSON handoff to a path supplied by NACC. Verify every claimed file, commit, and command independently.

## 7.4 Copilot

Implement stable programmatic CLI mode first. Add ACP mode only when the installed version passes contract tests. Because ACP settings such as effort or available tools may be server-startup options, launch separate supervised instances for profiles that require distinct settings.

Use Copilot primarily for GitHub-aware review and CI/CD investigation. Do not allow it to bypass GitHub branch or environment protections.

## 7.5 OpenCode and external gateways

Use OpenCode as the common integration for TokenRouter, B.AI, and configured open-source model gateways.

Support base URL, protocol, secret reference, headers, model discovery, user-defined model IDs, reasoning/thinking parameter mapping, concurrency, timeout, and user-entered pricing metadata.

Do not assume any exact “Qwen/Quan,” GLM, or DeepSeek model name. Display the exact provider-returned model IDs and permit aliases.

---

# 8. GUI requirements

The GUI is not a terminal wrapper. Build a coherent control center with these production pages:

1. First-run Setup Wizard
2. Dashboard
3. Projects
4. Providers and Accounts
5. Model Catalog
6. Role Matrix
7. Workflow Designer
8. Live Run Center
9. Agent Sessions/Terminals
10. Worktree Manager
11. Review Center
12. Quality Gates
13. CI/CD Center
14. Usage and Quotas
15. Security and Policies
16. Audit Log
17. Diagnostics and Updates

## 8.1 Role Matrix

Every role must be GUI-configurable for:

- provider;
- account;
- exact model or capability alias;
- reasoning: Auto, Off, Minimal, Low, Medium, High, Extra High, Maximum;
- thinking: Auto, On, Off, Managed, Unsupported;
- permission profile;
- native Windows/PowerShell/WSL2/Docker runtime;
- max turns/time;
- concurrency;
- network;
- tools;
- MCP/plugin profile;
- fallback chain;
- retry policy;
- budget;
- approval policy.

Show requested settings and effective provider-native settings before a run starts. Block invalid combinations. Never silently downgrade.

## 8.2 Workflow Designer

Implement a visual, versioned DAG editor with nodes, dependencies, parallel branches, retries, fallbacks, approvals, quality gates, input contracts, and output artifacts. Include validated presets before exposing unrestricted custom graphs.

## 8.3 Live Run Center

Display the graph, actual provider/model, current status, worktree, process, terminal/events, files changed, tool approvals, quality results, fallbacks, usage, and pause/cancel controls.

## 8.4 UX quality

The application must be usable by a non-expert through presets. Advanced options should be available but not overwhelm the default flow. Ensure responsive layout, keyboard access, visible focus, accessible labels, adequate contrast, loading/error/empty states, and no invisible hover-only controls.

Perform browser/webview visual QA and Windows packaged-app smoke testing rather than relying only on unit tests.

---

# 9. Workflow and state-machine requirements

Implement a durable, event-backed workflow engine with states equivalent to:

```text
Draft → Preflight → Exploring → Planning → Approval → Allocating
→ Implementing → LocalVerification → Reviewing → Repairing
→ Integrating → PullRequest → ContinuousIntegration → Staging
→ ProductionApproval → Deploying → Completed
```

Also support Failed, Cancelled, Superseded, and RequiresManualIntervention.

Requirements:

- versioned DAG templates;
- transactional state transitions;
- node attempts and leases;
- provider/account/model concurrency semaphores;
- local resource limits;
- pause/cancel/resume;
- retry and visible fallback;
- no unbounded loops;
- restart recovery and reconciliation;
- idempotent external actions;
- explicit completion evidence.

Default automated repair limit: two attempts for the same failure signature, followed by root-cause reassessment or manual intervention.

---

# 10. Process and Windows requirements

- Use typed executable and argument arrays.
- Avoid interpolated shell commands.
- Use ConPTY/portable PTY for interactive sessions.
- Contain native process trees in Windows Job Objects.
- Implement graceful cancellation before forced termination.
- Track PIDs, child trees, provider session IDs, worktree leases, and timestamps.
- Reconcile orphans after restart.
- Handle spaces, Unicode, long paths, drive letters, UNC paths, junctions, and case-insensitive comparisons safely.
- Bound all stream buffers and apply backpressure.
- Page persisted logs rather than loading them all into memory.
- Support WSL2 and optional Docker through a runtime abstraction, not scattered command conditionals.

---

# 11. Worktree and Git requirements

- Every write-enabled parallel worker gets an independent branch and worktree.
- The user's primary checkout stays untouched by default.
- Record base commit and detect drift.
- Enforce path ownership and forbidden paths from the task contract.
- Reviewers must not edit the implementation worktree.
- Integrate verified commits serially in a dedicated integration worktree.
- Never delete a worktree with uncommitted or unpushed evidence without a deliberate user action.
- Quarantine ambiguous/failed worktrees.
- Respect Git hooks, signing, LFS, credential helpers, and repository configuration.
- Preserve unrelated user changes.

---

# 12. Security requirements

Develop a threat model before enabling autonomous write mode.

At minimum address:

- malicious repository instructions;
- prompt injection in source files, issues, logs, webpages, and terminal output;
- command injection;
- path traversal and junction escapes;
- secret exfiltration;
- destructive Git operations;
- workflow/CI privilege escalation;
- malicious MCP server/tool;
- provider output spoofing;
- updater compromise;
- log/crash-report leakage;
- webview remote content;
- orphan process abuse;
- compromised external gateway.

Implement:

- strict Tauri capabilities and scopes;
- strict CSP;
- typed Rust commands;
- policy checks before every privileged operation;
- Windows Credential Manager for NACC-owned secrets;
- environment allowlists;
- command/path/network policies;
- secret redaction;
- approval gates;
- signed updater;
- audit trail;
- temporary, expiring danger mode only.

Never grant arbitrary shell access to the frontend.

---

# 13. Structured task contracts and handoffs

Create versioned JSON Schemas for:

- task contract;
- agent handoff;
- review result;
- quality-gate result;
- CI failure diagnosis.

Every worker receives bounded scope, owned paths, forbidden paths, acceptance criteria, required evidence, and the exact output path/schema.

Validate every handoff in Rust. Cross-check:

- files against Git diff;
- commits against Git;
- tests against captured command records;
- CI state against GitHub;
- claimed settings against the actual launch profile.

Do not pass enormous raw conversations between agents. Pass structured summaries, relevant files, decisions, and evidence.

---

# 14. Quality and review requirements

Implement deterministic project quality profiles for applicable formatter, lint, static analysis, typecheck, unit, integration, contract, browser, accessibility, security, dependency, migration, build, packaging, and smoke-test commands.

Capture command, working directory, environment policy, exit code, duration, redacted output, parsed results, and artifact references.

Use a different provider family for review where configured. Show line-level findings with severity, evidence, disposition, and repair link.

Retries may gather flakiness evidence but must not erase the initial failure.

---

# 15. GitHub and CI/CD requirements

Use GitHub Actions for deterministic CI/CD.

NACC should:

- create or update agent branches;
- open draft pull requests;
- read checks, workflow runs, jobs, steps, logs, annotations, artifacts, environments, and approvals;
- create a dedicated CI repair workflow/worktree;
- classify failures;
- rerun only permitted workflows/jobs;
- show staging and production status.

Required CI failure classes:

- production defect;
- stale test;
- test isolation;
- flaky test;
- environment/runner;
- dependency/registry;
- workflow/configuration;
- cache;
- secret/permission;
- external service;
- timeout/resources;
- platform-specific.

A rerun alone is not a fix. Require evidence and classification.

Production remains approval-gated. Do not store deployment secrets in NACC prompts or logs.

---

# 16. Persistence, recovery, and audit

Use SQLite with embedded, versioned Rust migrations. Test upgrades from every supported prior schema.

Persist:

- provider profiles and capability snapshots;
- role and workflow versions;
- runs, nodes, attempts, and events;
- worktree/process leases;
- approvals and policy decisions;
- handoffs and quality evidence;
- review findings;
- GitHub references;
- usage observations;
- audit events.

On startup, reconcile incomplete runs, process trees, worktrees, provider sessions, and GitHub state before offering resume.

Do not persist hidden reasoning. Do persist visible plans, summaries, tool actions, outputs, commands, and evidence.

---

# 17. Implementation sequence

Work in vertical slices and keep the application runnable after each major phase.

## Phase 0: Audit and ADR

No broad edits. Complete foundation decision with evidence.

## Phase 1: Hardened Tauri/Rust foundation

Upgrade/pin Tauri 2 only after compatibility verification. Establish modular workspace, typed IPC, strict capabilities, CSP, tracing, error handling, and reproducible builds.

## Phase 2: Domain/storage/events

Implement schema, migrations, repositories, workflow/event records, and audit foundations.

## Phase 3: Process/runtime/worktree

Implement Job Objects, PTY streams, cancellation, WSL abstraction, safe Git, leases, and recovery.

## Phase 4: Provider framework

Implement provider contract, capability snapshots, normalized events, health, fixtures, and contract tests.

## Phase 5: Claude + Codex vertical slice

Demonstrate read-only exploration, planning, isolated implementation, deterministic test, review, diff, and local commit.

## Phase 6: Setup Wizard + Role Matrix

Deliver complete GUI configuration, validation, aliases, exact models, reasoning/thinking, permissions, fallbacks, and effective settings.

## Phase 7: Durable DAG orchestration

Deliver templates, concurrency, checkpoints, approvals, pause/cancel/resume, retries, visible fallbacks, and crash recovery.

## Phase 8: Antigravity + OpenCode

Add native/WSL Antigravity and gateway profiles for TokenRouter/B.AI with schema-valid handoffs.

## Phase 9: Review + quality

Add diff center, independent reviews, deterministic quality evidence, and bounded repair.

## Phase 10: GitHub + Copilot CI/CD

Add PR/Actions UI, stable Copilot CLI, optional version-gated ACP, CI diagnosis, repair runs, staging, and production approvals.

## Phase 11: Security/reliability hardening

Complete threat model, policy tests, redaction, secret storage, abuse cases, orphan recovery, migration testing, and signed updates.

## Phase 12: Windows release

Create signed installer/update artifacts, clean-machine smoke tests, complete documentation, backup/restore, and release checklist.

Do not mark the whole product complete after only building screens or mocked adapters.

---

# 18. Verification commands and gates

Use repository-native commands, but establish at least these classes of checks:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
frontend format/lint/typecheck/unit tests
frontend production build
Tauri development smoke launch
Tauri production build
Rust integration tests
provider adapter fixture tests
SQLite migration tests
worktree/process cancellation tests
Windows packaged-app smoke test
```

Run focused tests during implementation and the complete relevant suite before declaring a phase complete.

Where a command is too costly locally, create a GitHub Actions job and inspect the actual result. Do not claim it passed without evidence.

---

# 19. Documentation deliverables

Maintain throughout implementation:

- architecture overview;
- ADRs;
- provider adapter contracts and tested versions;
- threat model;
- permission model;
- database schema/migrations;
- workflow state machine;
- process/recovery design;
- worktree lifecycle;
- CI/CD integration;
- installer/updater signing;
- user guide;
- troubleshooting;
- backup/restore;
- upstream delta;
- release checklist.

Documentation must reflect the implemented system, not aspirational features.

---

# 20. Completion evidence

Before final completion, demonstrate at least one real end-to-end scenario on a safe test repository:

1. Add project through GUI.
2. Detect and authenticate at least Claude and Codex through native mechanisms.
3. Discover actual models/capabilities.
4. Assign roles through Role Matrix.
5. Start a workflow.
6. Run parallel read-only explorers.
7. Generate an architecture/implementation contract.
8. Allocate separate worktrees.
9. Implement a change.
10. Run deterministic local quality gates.
11. Review with a different provider.
12. Repair a seeded finding.
13. Integrate serially.
14. Open a draft GitHub pull request.
15. Observe GitHub Actions.
16. Diagnose a seeded CI failure and create a repair task.
17. Pass CI.
18. Reach staging or a simulated staging gate.
19. Stop at explicit production approval.
20. Restart NACC during a controlled run and prove safe recovery.

Also demonstrate:

- cancellation kills the complete process tree;
- unsupported reasoning control is visibly disabled;
- a rate-limit fallback is visible and audited;
- secrets are redacted;
- an agent is blocked from a protected path/operation;
- a dirty worktree is quarantined rather than destroyed;
- the signed Windows package installs on a clean machine.

---

# 21. Operational behavior while you work

- Analyze before editing.
- Keep a concise implementation log.
- Make small, reviewable commits by phase.
- Do not push, publish releases, alter repository visibility, deploy production, or change organization security unless explicitly authorized.
- Preserve all unrelated work.
- When an assumption can be verified from code, CLI help, official docs, tests, or runtime behavior, verify it.
- When a feature is blocked by an external provider limitation, implement an honest degraded mode and document it; do not fake support.
- Prefer one correct implementation over duplicated provider-specific shortcuts.
- Fix root causes, not only symptoms.
- Do not endlessly iterate. Use acceptance criteria and evidence.

---

# 22. Final response format

At the end, report:

1. Foundation decision and exact upstream commit.
2. Architecture implemented.
3. Provider adapters and tested CLI versions.
4. GUI pages completed.
5. Security controls completed.
6. Tests and builds run with exact results.
7. End-to-end demonstration evidence.
8. Remaining limitations tied to external provider contracts.
9. Files changed and important ADRs.
10. Installer/update artifact locations.
11. Any actions deliberately not performed, such as production deployment.

Do not claim “100% bug-free,” “zero possibility of errors,” or complete support where evidence is absent. Completion means the documented acceptance criteria pass with reproducible evidence.

Begin now by reading the entire master plan, auditing the selected foundation, and producing the Phase 0 evidence and ADR before modifying the architecture.

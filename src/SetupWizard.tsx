import { useEffect, useRef, useState } from "react";
import {
  commands,
  type CapabilitySnapshotView,
  type PrerequisiteView,
  type ProviderInstallationView,
  type WorkspaceCheckView,
} from "./bindings";
import "./SetupWizard.css";

const supported = ["claude", "codex"] as const;
const message = (error: unknown) => error instanceof Error ? error.message : String(error);

/// First-run Setup Wizard (master plan S17.1). Rendered as one scrollable,
/// sectioned panel because every step is independently re-runnable and none
/// blocks another -- a modal step-machine would hide already-collected
/// facts behind back-navigation. Steps the spec names that this build cannot
/// perform are listed in step 8 with the reason, never silently omitted.
export default function SetupWizard() {
  const [prerequisites, setPrerequisites] = useState<PrerequisiteView[]>([]);
  const [installations, setInstallations] = useState<ProviderInstallationView[]>([]);
  const [auths, setAuths] = useState<Record<string, string>>({});
  const [caps, setCaps] = useState<Record<string, CapabilitySnapshotView | null>>({});
  const [workspacePath, setWorkspacePath] = useState("");
  const [workspace, setWorkspace] = useState<WorkspaceCheckView | null>(null);
  const [starterNote, setStarterNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(false);
  const operation = useRef(false);

  useEffect(() => {
    mounted.current = true;
    void (async () => {
      const result = await commands.listProviderInstallations();
      if (!mounted.current) return;
      if (result.status === "ok") setInstallations(result.data);
    })();
    return () => { mounted.current = false; };
  }, []);

  async function run(action: () => Promise<void>) {
    if (operation.current) return;
    operation.current = true;
    setBusy(true);
    setError(null);
    try { await action(); }
    catch (err) { if (mounted.current) setError(message(err)); }
    finally { operation.current = false; if (mounted.current) setBusy(false); }
  }

  return <section className="setup-wizard role-matrix" aria-labelledby="setup-wizard-title">
    <h2 id="setup-wizard-title">Setup Wizard</h2>
    <p>First-run onboarding, one re-runnable step at a time. Nothing here installs software, reads credential
      contents, or starts an agent session.</p>
    {error && <p role="alert">{error}</p>}

    <h3>Step 1 — Welcome and local security explanation</h3>
    <p>NACC runs agent CLIs on this machine under the permission profiles you assign. Privilege is scoped per role
      and per run, integration and CI operations are approval-gated, credentials stay in each provider's own native
      store (NACC checks their presence, never their contents), and everything NACC does is recorded in the local
      database.</p>

    <h3>Step 2 — Detect prerequisites</h3>
    <div className="role-actions">
      <button type="button" disabled={busy} onClick={() => void run(async () => {
        const result = await commands.checkPrerequisites({ include_gh: true });
        if (result.status === "error") throw new Error(result.error);
        setPrerequisites(result.data);
      })}>{busy ? "Checking…" : "Check git and gh"}</button>
    </div>
    {prerequisites.map(row => <p key={row.name} data-testid={`prereq-${row.name}`}>
      <strong>{row.name}</strong>: {row.detected ? `detected — ${row.version}` : "not detected"}{row.detail ? ` — ${row.detail}` : null}
    </p>)}

    <h3>Step 3 — Agent CLIs and exact versions</h3>
    {installations.length === 0
      ? <p>No saved detections. Use the Detect buttons in the Providers panel, then reload here.</p>
      : <ul>{installations.map(row => <li key={`${row.provider}-${row.runtime}`}>
        <strong>{row.provider}</strong> ({row.runtime}): {row.installed ? `version ${row.version ?? "unreported"}` : "not detected"}
      </li>)}</ul>}

    <h3>Steps 4–6 — Installation guidance, native sign-in, verification</h3>
    <p>NACC never installs a provider CLI on its own. Install each CLI by its vendor's documented route, sign in
      through the vendor's own native flow, then use the buttons below (or the Providers panel) to verify —
      sign-in verification reads only the credential store's presence.</p>
    <div className="role-actions">
      {supported.map(provider => <button key={provider} type="button" disabled={busy} onClick={() => void run(async () => {
        const result = await commands.checkProviderAuth({ provider_id: provider });
        if (result.status === "error") throw new Error(result.error);
        setAuths(current => ({ ...current, [provider]: `${result.data.authenticated ? "signed in" : "not signed in"}${result.data.detail ? ` — ${result.data.detail}` : ""}` }));
      })}>{busy ? "Checking…" : `Verify ${provider} sign-in`}</button>)}
    </div>
    {Object.entries(auths).map(([provider, state]) => <p key={provider}><strong>{provider}</strong>: {state}</p>)}

    <h3>Step 7 — Discover models and capabilities</h3>
    <div className="role-actions">
      {supported.map(provider => <button key={provider} type="button" disabled={busy} onClick={() => void run(async () => {
        const result = await commands.probeProviderCapabilities({ provider_id: provider });
        if (result.status === "error") throw new Error(result.error);
        setCaps(current => ({ ...current, [provider]: result.data }));
      })}>{busy ? "Probing…" : `Probe ${provider} capabilities`}</button>)}
    </div>
    {Object.entries(caps).map(([provider, cap]) => cap
      ? <p key={provider}><strong>{provider}</strong>: {cap.models.length} model(s) reported, health {cap.health}. Saved as a timestamped snapshot.</p>
      : null)}

    <h3>Step 12 — Validate a sample repository (read-only)</h3>
    <label>Workspace path (absolute)<input value={workspacePath} onChange={event => setWorkspacePath(event.target.value)} /></label>
    <div className="role-actions">
      <button type="button" disabled={busy || !workspacePath.trim()} onClick={() => void run(async () => {
        const result = await commands.checkWorkspace({ path: workspacePath.trim() });
        if (result.status === "error") throw new Error(result.error);
        setWorkspace(result.data);
      })}>{busy ? "Validating…" : "Validate workspace"}</button>
    </div>
    {workspace && <p data-testid="workspace-check">
      {workspace.path}: an existing{workspace.is_git_repo ? " git repository — ready for run workspaces and worktree isolation" : " directory, but not a git repository — runs there would have no worktree isolation"}.
    </p>}

    <h3>Step 11 — Create starter Role Matrix rows</h3>
    <p>Creates four <em>disabled, provider-unassigned</em> rows (explorer, planner, implementer, reviewer) so the
      Role Matrix starts from the master plan's default shape. Nothing is enabled or assigned until you do it.</p>
    <div className="role-actions">
      <button type="button" disabled={busy} onClick={() => void run(async () => {
        const roles = [
          ["Starter repository explorer", "repository_explorer", "read_only"],
          ["Starter architect planner", "architect_planner", "plan_only"],
          ["Starter backend implementer", "backend_implementer", "autonomous_worktree"],
          ["Starter general reviewer", "general_code_reviewer", "read_only"],
        ] as const;
        let created = 0;
        for (const [name, role_kind, permission_profile] of roles) {
          const result = await commands.createRoleProfile({
            name, role_kind, provider_id: null, model_id: null, thinking_mode: "auto",
            reasoning_level: "auto", permission_profile, account_label: null, fallbacks: [],
          });
          if (result.status === "error") throw new Error(result.error);
          created += 1;
        }
        setStarterNote(`${created} starter rows created (disabled, unassigned).`);
      })}>{busy ? "Creating…" : "Create starter rows"}</button>
    </div>
    {starterNote && <p role="status">{starterNote}</p>}

    <h3>Not wired in this build</h3>
    <p>Honest gaps, not hidden ones: launching a provider's native login flow (steps 4–5's launch half — verify-only
      is implemented), the harmless read-only smoke prompt (step 8 — needs a configured, enabled role and a live
      launch), local runtime preferences beyond native Windows (step 9), and secrets-storage configuration
      (step 10 — NACC-owned secrets are a Phase 11 deliverable). These stay visible here until they exist.</p>
  </section>;
}

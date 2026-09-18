import { useEffect, useRef, useState, type FormEvent } from "react";
import { commands, type CapabilitySnapshotView, type CreateRoleProfileArgs, type RoleKind, type RoleProfileView, type ProviderId } from "./bindings";
import "./RoleMatrix.css";

const roles: Exclude<RoleKind, object>[] = [
  "brain_main_orchestrator", "architect_planner", "repository_explorer", "external_researcher",
  "frontend_implementer", "backend_implementer", "database_migration_implementer", "test_engineer",
  "qa_reviewer", "general_code_reviewer", "security_reviewer", "accessibility_ux_reviewer",
  "performance_reviewer", "documentation_writer", "refactor_migration_specialist",
  "ci_cd_investigator", "integrator", "release_manager",
];
// Only adapters actually registered by the native runtime are selectable.
// Unsupported legacy values can still be rendered while editing so the
// user can repair old rows, but they are never offered as runnable choices.
const runnableProviders: ProviderId[] = ["claude", "codex"];
const permissions: CreateRoleProfileArgs["permission_profile"][] = [
  "read_only", "plan_only", "autonomous_worktree", "repository_maintainer", "ci_maintainer", "release_candidate",
];
const emptyProfile = (): CreateRoleProfileArgs => ({
  name: "", role_kind: "repository_explorer", provider_id: null, model_id: null,
  thinking_mode: "auto", reasoning_level: "auto", permission_profile: "read_only",
  account_label: null, fallbacks: [],
});
const label = (value: string) => value.replaceAll("_", " ");
const message = (error: unknown) => error instanceof Error ? error.message : String(error);
const isRunnableProvider = (provider: ProviderId): boolean => runnableProviders.includes(provider);

export default function RoleMatrix() {
  const [profiles, setProfiles] = useState<RoleProfileView[]>([]);
  const [caps, setCaps] = useState<Record<string, CapabilitySnapshotView | null>>({});
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState("");
  const [draft, setDraft] = useState(emptyProfile);
  const [editing, setEditing] = useState<RoleProfileView | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const nameInput = useRef<HTMLInputElement>(null);
  const operation = useRef(false);
  const mounted = useRef(false);
  const loadVersion = useRef(0);

  async function load() {
    const version = ++loadVersion.current;
    setLoading(true);
    setError(null);
    try {
      const result = await commands.listRoleProfiles();
      if (!mounted.current || version !== loadVersion.current) return;
      if (result.status === "error") throw new Error(result.error);
      setProfiles(result.data);
    } catch (err) {
      if (mounted.current && version === loadVersion.current) setError(message(err));
    } finally {
      if (mounted.current && version === loadVersion.current) setLoading(false);
    }
  }
  useEffect(() => {
    mounted.current = true;
    void load();
    return () => { mounted.current = false; };
  }, []);

  // Model-aware controls need a persisted capability snapshot to be honest:
  // fetch the latest one for the chosen provider whenever it changes. No
  // snapshot (or an unregistered provider) means the thinking/reasoning
  // controls stay disabled with an explanation -- never a guessed default
  // (master plan S10.1/S10.2, acceptance 11).
  const provider = typeof draft.provider_id === "string" ? draft.provider_id : null;
  useEffect(() => {
    if (!provider || !isRunnableProvider(provider)) return;
    let cancelled = false;
    void (async () => {
      const result = await commands.latestProviderCapabilities({ provider_id: provider as ProviderId });
      if (cancelled) return;
      setCaps(current => ({ ...current, [provider]: result.status === "ok" ? result.data : null }));
    })();
    return () => { cancelled = true; };
  }, [provider]);

  const fallbackProviders = [...new Set(draft.fallbacks.map(fallback => fallback.provider_id)
    .filter(isRunnableProvider))];
  const fallbackProviderKey = fallbackProviders.join("|");
  useEffect(() => {
    if (!fallbackProviderKey) return;
    let cancelled = false;
    void Promise.all(fallbackProviderKey.split("|").map(async providerId => {
      const fallbackProvider = providerId as ProviderId;
      const result = await commands.latestProviderCapabilities({ provider_id: fallbackProvider });
      if (cancelled) return;
      setCaps(current => ({ ...current, [fallbackProvider]: result.status === "ok" ? result.data : null }));
    }));
    return () => { cancelled = true; };
  }, [fallbackProviderKey]);

  const verifiedModels = provider && caps[provider]?.models ? caps[provider]!.models : [];
  const modelCaps = provider && draft.model_id && caps[provider]
    ? caps[provider]!.models.find(model => model.id === draft.model_id)
    : undefined;
  const reasoningChoices = modelCaps?.reasoning_levels ?? [];
  const reasoningVerified = reasoningChoices.length > 0;
  const thinkingManaged = modelCaps !== undefined
    && (modelCaps.thinking === "managed_by_provider" || modelCaps.thinking === "unsupported");

  async function mutate(action: () => Promise<void>) {
    if (operation.current) return;
    operation.current = true;
    setBusy(true);
    setError(null);
    setNotice("");
    try { await action(); }
    catch (err) { if (mounted.current) setError(message(err)); }
    finally {
      operation.current = false;
      if (mounted.current) setBusy(false);
    }
  }
  function reset() {
    setEditing(null);
    setDraft(emptyProfile());
  }
  async function save(event: FormEvent) {
    event.preventDefault();
    if (!draft.name.trim() || (typeof draft.role_kind === "object" && !draft.role_kind.custom.trim())) {
      setError("Profile name and custom role name must not be blank.");
      return;
    }
    if (provider && !isRunnableProvider(provider)) {
      setError(`Provider ${provider} is not runnable in this build. Select Claude, Codex, or Unassigned.`);
      return;
    }
    if (draft.model_id && !provider) {
      setError("An explicit model requires a primary provider.");
      return;
    }
    if (draft.model_id && !modelCaps) {
      setError(`Model ${draft.model_id} is not verified for ${provider}. Run “Check capabilities” in Providers and select a verified model.`);
      return;
    }
    if (reasoningVerified && !reasoningChoices.includes(draft.reasoning_level)) {
      // S10.1: "If the selected model does not support the requested value,
      // block save" -- never silently downgrade.
      setError(`Model ${draft.model_id} does not list reasoning level "${label(draft.reasoning_level)}". Supported: ${reasoningChoices.map(label).join(", ")}.`);
      return;
    }
    for (const [index, fallback] of draft.fallbacks.entries()) {
      if (!isRunnableProvider(fallback.provider_id)) {
        setError(`Fallback ${index + 1} provider ${fallback.provider_id} is not runnable in this build.`);
        return;
      }
      if (fallback.model_id) {
        const snapshot = caps[fallback.provider_id];
        const verified = snapshot?.models.some(model => model.id === fallback.model_id) ?? false;
        if (!verified) {
          setError(`Fallback ${index + 1} model ${fallback.model_id} is not verified for ${fallback.provider_id}. Run “Check capabilities” first.`);
          return;
        }
      }
    }
    await mutate(async () => {
      const args: CreateRoleProfileArgs = {
        name: draft.name.trim(), role_kind: draft.role_kind, provider_id: draft.provider_id,
        model_id: draft.model_id?.trim() || null, thinking_mode: draft.thinking_mode,
        reasoning_level: draft.reasoning_level, permission_profile: draft.permission_profile,
        account_label: draft.account_label?.trim() || null,
        fallbacks: draft.fallbacks.map(fallback => ({
          ...fallback, reason: fallback.reason.trim() || "role fallback chain",
        })),
      };
      const result = editing
        ? await commands.updateRoleProfile(editing.id, { ...args, enabled: editing.enabled })
        : await commands.createRoleProfile(args);
      if (result.status === "error") throw new Error(result.error);
      if (!mounted.current) return;
      setProfiles(current => editing
        ? current.map(profile => profile.id === editing.id ? result.data.profile : profile)
        : [...current, result.data.profile]);
      setNotice("Role profile saved. Provider and explicit model assignments were validated against runnable adapters and verified capability snapshots.");
      reset();
    });
  }

  return <section className="role-matrix" aria-labelledby="role-matrix-title">
    <h2 id="role-matrix-title">Role Matrix</h2>
    <p>Save independent role assignments in the local database. Saving does not launch an agent or grant permissions.</p>
    <p id="capability-note">Thinking and reasoning controls stay disabled until a capability snapshot for the chosen
      provider and model verifies what it supports — run “Check capabilities” in the Providers panel. A requested
      level a verified model does not list is refused at save, never silently downgraded.</p>
    {error && <p role="alert">{error}</p>}
    <p role="status">{notice}</p>
    <button type="button" disabled={busy || loading} onClick={() => void load()}>Refresh profiles</button>
    {loading ? <p role="status">Loading role profiles…</p> : <>
      {profiles.length === 0 ? <p>No role profiles saved.</p> : <div className="role-table-wrap">
        <table><caption>Saved role assignments</caption>
          <thead><tr><th scope="col">Name / role</th><th scope="col">Provider / model</th><th scope="col">Status</th><th scope="col">Actions</th></tr></thead>
          <tbody>{profiles.map(profile => <tr key={profile.id}>
            <th scope="row">{profile.name}<small>{typeof profile.role_kind === "string" ? label(profile.role_kind) : profile.role_kind.custom}</small></th>
            <td>{profile.provider_id ?? "Unassigned"}<small>{profile.model_id ?? "Provider default"}</small></td>
            <td>{profile.enabled ? "Enabled" : "Disabled"}</td>
            <td><div className="role-actions">
              <button type="button" disabled={busy} aria-label={`Edit ${profile.name}`} onClick={() => {
                setEditing(profile); setDraft({ ...profile }); setError(null); setNotice(""); nameInput.current?.focus();
              }}>Edit</button>
              <button type="button" disabled={busy || editing?.id === profile.id} aria-label={`${profile.enabled ? "Disable" : "Enable"} ${profile.name}`} onClick={() => void mutate(async () => {
                const result = await commands.setRoleProfileEnabled(profile.id, !profile.enabled);
                if (result.status === "error") throw new Error(result.error);
                if (!mounted.current) return;
                setProfiles(current => current.map(row => row.id === profile.id ? { ...row, enabled: !row.enabled } : row));
                setNotice("Profile status saved.");
              })}>{profile.enabled ? "Disable" : "Enable"}</button>
              {deleting !== profile.id ? <button type="button" disabled={busy} aria-label={`Delete ${profile.name}`} onClick={() => setDeleting(profile.id)}>Delete</button> : <>
                <span>Delete {profile.name} permanently?</span>
                <button type="button" disabled={busy} onClick={() => void mutate(async () => {
                  const result = await commands.deleteRoleProfile(profile.id);
                  if (result.status === "error") throw new Error(result.error);
                  if (!mounted.current) return;
                  setProfiles(current => current.filter(row => row.id !== profile.id));
                  if (editing?.id === profile.id) reset();
                  setDeleting(null); setNotice(result.data ? "Profile deleted." : "Profile was already deleted.");
                })}>Confirm delete</button>
                <button type="button" disabled={busy} onClick={() => setDeleting(null)}>Keep profile</button>
              </>}
            </div></td>
          </tr>)}</tbody>
        </table>
      </div>}
    </>}
    <form onSubmit={save}>
      <h3>{editing ? `Edit ${editing.name}` : "New role profile"}</h3>
      <fieldset disabled={busy || loading}>
        <legend>Requested configuration</legend>
        <label>Profile name<input ref={nameInput} required value={draft.name} onChange={event => setDraft({ ...draft, name: event.target.value })} /></label>
        <label>Role<select value={typeof draft.role_kind === "string" ? draft.role_kind : "custom"} onChange={event => setDraft({ ...draft, role_kind: event.target.value === "custom" ? { custom: "" } : event.target.value as RoleKind })}>
          {roles.map(role => <option key={role} value={role}>{label(role)}</option>)}<option value="custom">Custom role</option>
        </select></label>
        {typeof draft.role_kind === "object" && <label>Custom role name<input required value={draft.role_kind.custom} onChange={event => setDraft({ ...draft, role_kind: { custom: event.target.value } })} /></label>}
        <label>Provider<select value={draft.provider_id ?? ""} onChange={event => setDraft({
          ...draft,
          provider_id: event.target.value ? event.target.value as ProviderId : null,
          model_id: null,
          thinking_mode: "auto",
          reasoning_level: "auto",
        })}>
          <option value="">Unassigned</option>
          {provider && !isRunnableProvider(provider) && <option value={provider} disabled>{provider} — unavailable in this build</option>}
          {runnableProviders.map(runnable => <option key={runnable} value={runnable}>{runnable}</option>)}
        </select></label>
        <label>Requested model ID<select aria-describedby="capability-note" value={draft.model_id ?? ""} onChange={event => setDraft({
          ...draft,
          model_id: event.target.value || null,
          thinking_mode: "auto",
          reasoning_level: "auto",
        })}>
          <option value="">Provider default</option>
          {draft.model_id && !verifiedModels.some(model => model.id === draft.model_id)
            && <option value={draft.model_id} disabled>{draft.model_id} — unverified legacy value</option>}
          {verifiedModels.map(model => <option key={model.id} value={model.id}>{model.display_name}</option>)}
        </select></label>
        {thinkingManaged
          ? <label>Thinking<select disabled aria-describedby="capability-note" value={modelCaps!.thinking}><option value={modelCaps!.thinking}>{label(modelCaps!.thinking)} — per this model's verified capabilities</option></select></label>
          : <label>Thinking<select disabled={!modelCaps} aria-describedby="capability-note" value={draft.thinking_mode} onChange={event => setDraft({ ...draft, thinking_mode: event.target.value as CreateRoleProfileArgs["thinking_mode"] })}>
            {modelCaps
              ? ["auto", "on", "off"].map(mode => <option key={mode} value={mode}>{label(mode)}</option>)
              : <option value={draft.thinking_mode}>{label(draft.thinking_mode)} — no verified capability snapshot for this provider/model</option>}
          </select></label>}
        <label>Reasoning effort<select disabled={!reasoningVerified} aria-describedby="capability-note" value={draft.reasoning_level} onChange={event => setDraft({ ...draft, reasoning_level: event.target.value as CreateRoleProfileArgs["reasoning_level"] })}>
          {reasoningVerified
            ? reasoningChoices.map(level => <option key={level} value={level}>{label(level)}</option>)
            : <option value={draft.reasoning_level}>{label(draft.reasoning_level)} — no verified capability snapshot for this provider/model</option>}
        </select></label>
        <label>Requested permission profile<select value={draft.permission_profile} onChange={event => setDraft({ ...draft, permission_profile: event.target.value as CreateRoleProfileArgs["permission_profile"] })}>
          {permissions.map(permission => <option key={permission} value={permission}>{label(permission)}</option>)}
        </select></label>
        <label>Account label (display only — never a credential)<input value={draft.account_label ?? ""} onChange={event => setDraft({ ...draft, account_label: event.target.value || null })} /></label>
        <div className="fallback-editor">
          <h4>Fallback chain</h4>
          <p>Consulted only when this row has no primary provider and the node declares none of its own. Every fallback actually taken is recorded on the attempt.</p>
          {draft.fallbacks.map((fallback, index) => <div className="role-actions" key={index}>
            <select aria-label={`Fallback ${index + 1} provider`} value={fallback.provider_id} onChange={event => {
              const fallbacks = [...draft.fallbacks];
              fallbacks[index] = { ...fallback, provider_id: event.target.value as ProviderId, model_id: null };
              setDraft({ ...draft, fallbacks });
            }}>
              {!isRunnableProvider(fallback.provider_id)
                && <option value={fallback.provider_id} disabled>{fallback.provider_id} — unavailable in this build</option>}
              {runnableProviders.map(p => <option key={p} value={p}>{p}</option>)}
            </select>
            <select aria-label={`Fallback ${index + 1} model (optional)`} value={fallback.model_id ?? ""} onChange={event => {
              const fallbacks = [...draft.fallbacks];
              fallbacks[index] = { ...fallback, model_id: event.target.value || null };
              setDraft({ ...draft, fallbacks });
            }}>
              <option value="">Provider default</option>
              {fallback.model_id && !caps[fallback.provider_id]?.models.some(model => model.id === fallback.model_id)
                && <option value={fallback.model_id} disabled>{fallback.model_id} — unverified legacy value</option>}
              {caps[fallback.provider_id]?.models.map(model => <option key={model.id} value={model.id}>{model.display_name}</option>)}
            </select>
            <input aria-label={`Fallback ${index + 1} reason`} placeholder="Why this fallback" value={fallback.reason} onChange={event => {
              const fallbacks = [...draft.fallbacks];
              fallbacks[index] = { ...fallback, reason: event.target.value };
              setDraft({ ...draft, fallbacks });
            }} />
            <button type="button" aria-label={`Remove fallback ${index + 1}`} onClick={() => setDraft({ ...draft, fallbacks: draft.fallbacks.filter((_, i) => i !== index) })}>Remove</button>
          </div>)}
          <button type="button" onClick={() => setDraft({ ...draft, fallbacks: [...draft.fallbacks, { provider_id: "claude", model_id: null, reason: "" }] })}>
            Add fallback
          </button>
        </div>
        <div className="role-actions"><button type="submit">{busy ? "Saving…" : "Save profile"}</button>{editing && <button type="button" onClick={reset}>Cancel edit</button>}</div>
      </fieldset>
    </form>
  </section>;
}

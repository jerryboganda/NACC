import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  commands,
  type ApprovalView,
  type RunSnapshotView,
  type WorkflowEventView,
  type WorkflowRunView,
  type WorkflowTemplateView,
} from "./bindings";
import "./RunConsole.css";

const message = (error: unknown) => error instanceof Error ? error.message : String(error);

export default function RunConsole() {
  const [templates, setTemplates] = useState<WorkflowTemplateView[]>([]);
  const [runs, setRuns] = useState<WorkflowRunView[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState("");
  const [project, setProject] = useState("");
  const [workspace, setWorkspace] = useState("");
  const [templateName, setTemplateName] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<RunSnapshotView | null>(null);
  const [events, setEvents] = useState<WorkflowEventView[]>([]);
  const [detailLoading, setDetailLoading] = useState(false);
  const [pauseReason, setPauseReason] = useState("");
  const [cancelReason, setCancelReason] = useState("");
  const operation = useRef(false);
  const mounted = useRef(false);
  const loadVersion = useRef(0);
  const detailVersion = useRef(0);

  // Templates and the durable run list. list_workflow_templates is a sync
  // Rust command, so tauri-specta generates it *without* the
  // {status:"ok"|"error"} wrapper the async commands get: it resolves to the
  // bare array and rejects only on transport failure.
  async function load() {
    const version = ++loadVersion.current;
    setLoading(true);
    setError(null);
    try {
      const [templateList, runsResult] = await Promise.all([
        commands.listWorkflowTemplates(),
        commands.listWorkflowRuns(),
      ]);
      if (!mounted.current || version !== loadVersion.current) return;
      if (runsResult.status === "error") throw new Error(runsResult.error);
      setTemplates(templateList);
      setRuns(runsResult.data);
      setTemplateName(current => templateList.some(template => template.name === current)
        ? current
        : templateList[0]?.name ?? "");
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

  // The selected run's snapshot plus its normalized event trail, as one
  // refreshable unit -- the run keeps advancing in the background, so every
  // action re-opens the run rather than trusting stale state.
  async function openRun(runId: string) {
    const version = ++detailVersion.current;
    setSelectedId(runId);
    setDetailLoading(true);
    try {
      const [snapshotResult, eventsResult] = await Promise.all([
        commands.getWorkflowRun({ run_id: runId }),
        commands.listWorkflowEvents({ run_id: runId }),
      ]);
      if (!mounted.current || version !== detailVersion.current) return;
      if (snapshotResult.status === "error") throw new Error(snapshotResult.error);
      if (eventsResult.status === "error") throw new Error(eventsResult.error);
      setSnapshot(snapshotResult.data);
      setEvents(eventsResult.data);
    } catch (err) {
      if (mounted.current && version === detailVersion.current) setError(message(err));
    } finally {
      if (mounted.current && version === detailVersion.current) setDetailLoading(false);
    }
  }

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

  function refreshRun(runId: string) {
    return Promise.all([load(), openRun(runId)]);
  }

  function startRun(event: FormEvent) {
    event.preventDefault();
    if (!project.trim() || !workspace.trim() || !templateName) {
      setError("A project ID, an absolute workspace path, and a template are required before a run can start.");
      return;
    }
    void mutate(async () => {
      const result = await commands.startWorkflowRun({
        project_id: project.trim(),
        template_name: templateName,
        workspace: workspace.trim(),
      });
      if (result.status === "error") throw new Error(result.error);
      if (!mounted.current) return;
      setNotice(`Run ${result.data.run.id} started. It keeps running if this window closes.`);
      setProject("");
      setWorkspace("");
      await refreshRun(result.data.run.id);
    });
  }

  function pauseRun() {
    if (!selectedId) return;
    if (!pauseReason.trim()) {
      setError("A pause reason is required; it is written to the run's durable trail.");
      return;
    }
    void mutate(async () => {
      const result = await commands.pauseWorkflowRun({ run_id: selectedId, reason: pauseReason.trim() });
      if (result.status === "error") throw new Error(result.error);
      if (!mounted.current) return;
      setNotice("Run paused. Resume it from the run list.");
      setPauseReason("");
      await refreshRun(selectedId);
    });
  }

  function cancelRun() {
    if (!selectedId) return;
    if (!cancelReason.trim()) {
      setError("A cancellation reason is required; it is written to the run's durable trail.");
      return;
    }
    void mutate(async () => {
      const result = await commands.cancelWorkflowRun({ run_id: selectedId, reason: cancelReason.trim() });
      if (result.status === "error") throw new Error(result.error);
      if (!mounted.current) return;
      setNotice("Run cancelled.");
      setCancelReason("");
      await refreshRun(selectedId);
    });
  }

  function resumeRun() {
    if (!selectedId) return;
    void mutate(async () => {
      const result = await commands.resumeWorkflowRun({ run_id: selectedId });
      if (result.status === "error") throw new Error(result.error);
      if (!mounted.current) return;
      setNotice("Run resumed.");
      await refreshRun(selectedId);
    });
  }

  function decideApproval(approval: ApprovalView, approved: boolean, by: string, reason: string | null, resumeAfter: boolean) {
    if (!selectedId) return Promise.resolve();
    return mutate(async () => {
      const result = await commands.decideWorkflowApproval({
        run_id: selectedId,
        approval_id: approval.id,
        approved,
        by,
        reason,
        resume_after: approved ? resumeAfter : false,
      });
      if (result.status === "error") throw new Error(result.error);
      if (!mounted.current) return;
      setNotice(`Approval for ${approval.node_key} recorded.`);
      await refreshRun(selectedId);
    });
  }

  const selectedTemplate = templates.find(template => template.name === templateName);

  return <section className="run-console" aria-labelledby="run-console-title">
    <h2 id="run-console-title">Run Console</h2>
    <p>Start a built-in workflow and watch its durable run. Nodes launch real agent CLIs: a node refuses to
      start unless its role has a provider and a model in the Role Matrix and the run has an explicit workspace.</p>
    {error && <p role="alert">{error}</p>}
    <p role="status">{notice}</p>
    <button type="button" disabled={busy || loading} onClick={() => void load()}>Refresh runs</button>
    {loading ? <p role="status">Loading workflows…</p> : <>
      <form onSubmit={startRun}>
        <h3>Start a run</h3>
        <fieldset disabled={busy}>
          <legend>The workspace is the exact directory agents work in — never guessed</legend>
          <label>Project ID<input value={project} onChange={event => setProject(event.target.value)} /></label>
          <label>Workspace (absolute path)<input value={workspace} onChange={event => setWorkspace(event.target.value)} /></label>
          <label>Workflow template<select value={templateName} onChange={event => setTemplateName(event.target.value)}>
            {templates.map(template => <option key={template.name} value={template.name}>{template.name}</option>)}
          </select></label>
          <div className="role-actions"><button type="submit">{busy ? "Starting…" : "Start run"}</button></div>
        </fieldset>
      </form>
      {selectedTemplate && <p>
        <strong>{selectedTemplate.name}</strong> — {selectedTemplate.description}
        <small>Nodes: {selectedTemplate.node_keys.join(", ")}</small>
      </p>}
      {!loading && templates.length === 0 && <p>No workflow templates available.</p>}
      {runs.length === 0 ? <p>No workflow runs yet.</p> : <div className="run-table-wrap">
        <table><caption>Durable runs — closing NACC leaves a real interrupted run, not a ghost</caption>
          <thead><tr><th scope="col">Run</th><th scope="col">Template</th><th scope="col">State</th><th scope="col">Updated</th><th scope="col">Actions</th></tr></thead>
          <tbody>{runs.map(run => <tr key={run.id}>
            <th scope="row"><code>{run.id}</code><small>project {run.project_id}</small></th>
            <td>{run.template_name}{run.note && <small>{run.note}</small>}</td>
            <td>{run.state}</td>
            <td>{run.updated_at_millis}</td>
            <td><button type="button" disabled={busy} aria-label={`Open run ${run.id}`} onClick={() => void openRun(run.id)}>Open</button></td>
          </tr>)}</tbody>
        </table>
      </div>}
      {detailLoading ? <p role="status">Loading run…</p> : snapshot && <div className="run-detail">
        <h3>Run detail</h3>
        <p data-testid="run-state">{snapshot.run.state} — {snapshot.run.template_name}
          {snapshot.run.note ? ` — ${snapshot.run.note}` : ""}</p>
        <h4>Nodes</h4>
        <div className="run-table-wrap">
          <table>
            <thead><tr><th scope="col">Node</th><th scope="col">State</th><th scope="col">Attempts</th><th scope="col">Detail</th></tr></thead>
            <tbody>{snapshot.nodes.map(node => <tr key={node.id}>
              <th scope="row">{node.title}<small>{node.node_key}{node.requires_approval ? " — needs approval" : ""}</small></th>
              <td>{node.state}</td>
              <td>{node.attempts}</td>
              <td>{node.last_detail ?? "—"}</td>
            </tr>)}</tbody>
          </table>
        </div>
        <h4>Approvals</h4>
        {snapshot.approvals.length === 0 ? <p>No approval gates recorded.</p> : <ul className="approval-list">
          {snapshot.approvals.map(approval => <li key={approval.id}>
            {approval.decision === null
              ? <ApprovalCard approval={approval} busy={busy} onDecide={decideApproval} />
              : <p>{approval.node_key}: {approval.decision}<small>{approval.decided_at_millis}</small></p>}
          </li>)}
        </ul>}
        {!snapshot.finished && <div className="run-actions">
          <label>Pause reason<input value={pauseReason} onChange={event => setPauseReason(event.target.value)} /></label>
          <button type="button" disabled={busy} onClick={pauseRun}>Pause run</button>
          <label>Cancellation reason<input value={cancelReason} onChange={event => setCancelReason(event.target.value)} /></label>
          <button type="button" disabled={busy} onClick={cancelRun}>Cancel run</button>
          {snapshot.run.state === "awaiting_approval"
            ? <p>Resume is blocked while an approval gate is open — decide the approval above.</p>
            : <button type="button" disabled={busy} onClick={() => void resumeRun()}>Resume run</button>}
        </div>}
        <h4>Checkpoints</h4>
        {snapshot.checkpoints.length === 0 ? <p>No checkpoints recorded.</p> : <ul>
          {snapshot.checkpoints.map(checkpoint => <li key={checkpoint.sequence}>
            #{checkpoint.sequence} {checkpoint.state} — {checkpoint.detail} <small>{checkpoint.created_at_millis}</small>
          </li>)}
        </ul>}
        <h4>Events</h4>
        {events.length === 0 ? <p>No events recorded yet.</p> : <ul className="event-list">
          {events.map((event, index) => <li className="run-event" key={index}>
            <code>{event.event_type}</code> <small>{event.created_at_millis}</small>
            <pre>{event.payload_json}</pre>
          </li>)}
        </ul>}
      </div>}
    </>}
  </section>;
}

// One open approval gate with its own decision draft. `by` is required even
// for approval, and a rejection requires a reason -- mirrored here so the
// user sees the rule before the backend refuses.
function ApprovalCard({ approval, busy, onDecide }: {
  approval: ApprovalView;
  busy: boolean;
  onDecide: (approval: ApprovalView, approved: boolean, by: string, reason: string | null, resumeAfter: boolean) => Promise<void>;
}) {
  const [by, setBy] = useState("");
  const [reason, setReason] = useState("");
  const [resumeAfter, setResumeAfter] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const suffix = ` (${approval.node_key})`;

  function decide(approved: boolean) {
    if (!by.trim()) {
      setProblem("Recording who decided is required.");
      return;
    }
    if (!approved && !reason.trim()) {
      setProblem("A rejection must state a reason.");
      return;
    }
    setProblem(null);
    void onDecide(approval, approved, by.trim(), approved ? null : reason.trim(), resumeAfter);
  }

  return <div className="approval-card">
    <p><strong>{approval.node_key}</strong> — {approval.summary}</p>
    {problem && <p role="alert">{problem}</p>}
    <label>Decided by{suffix}<input value={by} onChange={event => setBy(event.target.value)} /></label>
    <label>Rejection reason{suffix}<input value={reason} onChange={event => setReason(event.target.value)} /></label>
    <label className="approval-resume">
      <input type="checkbox" checked={resumeAfter} onChange={event => setResumeAfter(event.target.checked)} />
      Resume the run after approving{suffix}
    </label>
    <div className="role-actions">
      <button type="button" disabled={busy} aria-label={`Approve ${approval.node_key}`} onClick={() => decide(true)}>Approve</button>
      <button type="button" disabled={busy} aria-label={`Reject ${approval.node_key}`} onClick={() => decide(false)}>Reject</button>
    </div>
  </div>;
}

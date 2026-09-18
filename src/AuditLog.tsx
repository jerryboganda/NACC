import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  commands,
  type AuditRecordView,
} from "./bindings";
import "./OperationalSurfaces.css";

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

function formatTimestamp(millis: string): string {
  if (!millis) return "—";
  const d = new Date(Number(millis));
  return Number.isNaN(d.getTime()) ? millis : d.toLocaleString();
}

function shortId(id: string | null): string {
  if (!id) return "—";
  return id.length > 8 ? `${id.slice(0, 8)}…` : id;
}

export default function AuditLog() {
  const [records, setRecords] = useState<AuditRecordView[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [runIdInput, setRunIdInput] = useState("");
  const [limit, setLimit] = useState(50);
  const mounted = useRef(false);
  const loadVersion = useRef(0);

  async function loadAuditRecords() {
    const version = ++loadVersion.current;
    setLoading(true);
    setError(null);
    try {
      const clampedLimit = Math.max(1, Math.min(500, Number(limit) || 50));
      const cleanRunId = runIdInput.trim() || null;
      const result = await commands.listAuditRecords({
        workflow_run_id: cleanRunId,
        limit: clampedLimit,
      });

      if (!mounted.current || version !== loadVersion.current) return;
      if (result.status === "error") throw new Error(result.error);
      setRecords(result.data);
    } catch (err) {
      if (mounted.current && version === loadVersion.current) {
        setError(`Failed to retrieve audit records: ${message(err)}`);
      }
    } finally {
      if (mounted.current && version === loadVersion.current) {
        setLoading(false);
      }
    }
  }

  useEffect(() => {
    mounted.current = true;
    void loadAuditRecords();
    return () => {
      mounted.current = false;
    };
  }, []);

  function handleSubmit(e: FormEvent) {
    e.preventDefault();
    void loadAuditRecords();
  }

  return (
    <section className="operational-surface" aria-labelledby="audit-log-title">
      <header className="surface-header">
        <h2 id="audit-log-title">Security & Audit Trail</h2>
        <p className="surface-note">
          Read-only compliance and audit records, ordered newest-first. Command arguments are
          redacted by secret policy before persistence; audit entries cannot be mutated or deleted.
        </p>
      </header>

      {error && (
        <div role="alert" className="surface-alert" data-testid="audit-error">
          {error}
        </div>
      )}

      <form className="surface-controls" onSubmit={handleSubmit} aria-label="Filter audit records">
        <label htmlFor="audit-run-id">
          Workflow Run ID (Optional)
          <input
            id="audit-run-id"
            data-testid="audit-run-id-input"
            type="text"
            placeholder="UUID (e.g. 5b6f2b2e...)"
            value={runIdInput}
            onChange={(e) => setRunIdInput(e.target.value)}
          />
        </label>

        <label htmlFor="audit-limit">
          Limit
          <select
            id="audit-limit"
            data-testid="audit-limit-select"
            value={limit}
            onChange={(e) => setLimit(Number(e.target.value))}
          >
            <option value={25}>25</option>
            <option value={50}>50</option>
            <option value={100}>100</option>
            <option value={250}>250</option>
            <option value={500}>500</option>
          </select>
        </label>

        <button
          type="submit"
          data-testid="audit-apply-filter-button"
          disabled={loading}
        >
          {loading ? "Refreshing..." : "Filter & Refresh"}
        </button>
      </form>

      {loading && !records.length && <p data-testid="audit-loading">Loading audit records...</p>}

      {!loading && records.length === 0 && (
        <div className="surface-empty" data-testid="audit-empty">
          No audit records found for the current query.
        </div>
      )}

      {records.length > 0 && (
        <div className="surface-table-wrap">
          <table className="surface-table" aria-label="Security audit trail">
            <caption>
              Recorded Security Audit Events ({records.length})
            </caption>
            <thead>
              <tr>
                <th scope="col">Timestamp</th>
                <th scope="col">Actor / Action</th>
                <th scope="col">Correlation (Run / Node)</th>
                <th scope="col">Provider & Model</th>
                <th scope="col">Policy & Reasoning</th>
                <th scope="col">Command & Redacted Args</th>
              </tr>
            </thead>
            <tbody>
              {records.map((record) => (
                <tr key={record.id} data-testid={`audit-row-${record.id}`}>
                  <td style={{ whiteSpace: "nowrap" }}>
                    {formatTimestamp(record.created_at_millis)}
                  </td>
                  <td>
                    <strong>{record.action}</strong>
                    <div style={{ opacity: 0.75 }}>by {record.actor}</div>
                  </td>
                  <td className="surface-mono">
                    {record.workflow_run_id && (
                      <div title={record.workflow_run_id}>
                        Run: {shortId(record.workflow_run_id)}
                      </div>
                    )}
                    {record.node_run_id && (
                      <div title={record.node_run_id}>
                        Node: {shortId(record.node_run_id)}
                      </div>
                    )}
                    {!record.workflow_run_id && !record.node_run_id && (
                      <span style={{ opacity: 0.5 }}>—</span>
                    )}
                  </td>
                  <td>
                    <div>
                      {record.actual_provider ?? record.requested_provider ?? "—"}
                    </div>
                    <div className="surface-mono" style={{ fontSize: "0.8rem", opacity: 0.8 }}>
                      {record.actual_model ?? record.requested_model ?? "—"}
                    </div>
                  </td>
                  <td>
                    {record.effective_permission_profile && (
                      <div>
                        <span className="surface-mono" style={{ fontSize: "0.8rem" }}>
                          {record.effective_permission_profile}
                        </span>
                      </div>
                    )}
                    {record.effective_reasoning_level && (
                      <div style={{ fontSize: "0.8rem", opacity: 0.75 }}>
                        reasoning: {record.effective_reasoning_level}
                      </div>
                    )}
                    {!record.effective_permission_profile && !record.effective_reasoning_level && (
                      <span style={{ opacity: 0.5 }}>—</span>
                    )}
                  </td>
                  <td>
                    {record.command_executable ? (
                      <div>
                        <span className="surface-mono" style={{ fontWeight: 600 }}>
                          {record.command_executable}
                        </span>
                        {record.redacted_arguments.length > 0 && (
                          <div
                            className="surface-mono"
                            style={{ fontSize: "0.8rem", opacity: 0.85, marginTop: "0.2rem" }}
                          >
                            {record.redacted_arguments.join(" ")}
                          </div>
                        )}
                      </div>
                    ) : (
                      <span style={{ opacity: 0.5 }}>—</span>
                    )}
                    {record.working_directory && (
                      <div
                        className="surface-mono"
                        style={{ fontSize: "0.75rem", opacity: 0.6, marginTop: "0.25rem" }}
                      >
                        cwd: {record.working_directory}
                      </div>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

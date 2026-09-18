import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  commands,
  type QualityGateResultView,
  type ReviewFindingView,
  type ReviewSeverityView,
} from "./bindings";
import "./OperationalSurfaces.css";

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

function formatTimestamp(millis: string): string {
  if (!millis) return "—";
  const date = new Date(Number(millis));
  return Number.isNaN(date.getTime()) ? millis : date.toLocaleString();
}

function formatDuration(millis: string): string {
  const value = Number(millis);
  if (!Number.isFinite(value)) return `${millis} ms`;
  if (value < 1_000) return `${millis} ms`;
  return `${(value / 1_000).toFixed(2)} s`;
}

function shortId(id: string | null): string {
  if (!id) return "—";
  return id.length > 8 ? `${id.slice(0, 8)}…` : id;
}

function qualityStatus(record: QualityGateResultView) {
  if (record.timed_out) return { label: "Timed out", className: "badge-fail" };
  return record.passed
    ? { label: "Passed", className: "badge-pass" }
    : { label: "Failed", className: "badge-fail" };
}

function severityClass(severity: ReviewSeverityView): string {
  switch (severity) {
    case "blocker":
      return "badge-blocker";
    case "major":
      return "badge-major";
    case "minor":
      return "badge-minor";
    case "note":
      return "badge-note";
  }
}

export default function QualityReviewCenter() {
  const [qualityRecords, setQualityRecords] = useState<QualityGateResultView[]>([]);
  const [reviewFindings, setReviewFindings] = useState<ReviewFindingView[]>([]);
  const [qualityError, setQualityError] = useState<string | null>(null);
  const [reviewError, setReviewError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [runIdInput, setRunIdInput] = useState("");
  const [nodeIdInput, setNodeIdInput] = useState("");
  const [limit, setLimit] = useState(50);
  const mounted = useRef(false);
  const loadVersion = useRef(0);

  async function loadEvidence() {
    const version = ++loadVersion.current;
    setLoading(true);
    setQualityError(null);
    setReviewError(null);
    setQualityRecords([]);
    setReviewFindings([]);

    const args = {
      workflow_run_id: runIdInput.trim() || null,
      node_run_id: nodeIdInput.trim() || null,
      limit: Math.max(1, Math.min(500, Number(limit) || 50)),
    };

    try {
      const [qualityResult, reviewResult] = await Promise.all([
        commands.listQualityGateResults(args),
        commands.listReviewFindings(args),
      ]);

      if (!mounted.current || version !== loadVersion.current) return;

      if (qualityResult.status === "error") {
        setQualityError(`Failed to retrieve quality-gate evidence: ${qualityResult.error}`);
      } else {
        setQualityRecords(qualityResult.data);
      }

      if (reviewResult.status === "error") {
        setReviewError(`Failed to retrieve review findings: ${reviewResult.error}`);
      } else {
        setReviewFindings(reviewResult.data);
      }
    } catch (error) {
      if (mounted.current && version === loadVersion.current) {
        const detail = message(error);
        setQualityError(`Failed to retrieve quality-gate evidence: ${detail}`);
        setReviewError(`Failed to retrieve review findings: ${detail}`);
      }
    } finally {
      if (mounted.current && version === loadVersion.current) {
        setLoading(false);
      }
    }
  }

  useEffect(() => {
    mounted.current = true;
    void loadEvidence();
    return () => {
      mounted.current = false;
    };
  }, []);

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    void loadEvidence();
  }

  return (
    <section className="operational-surface" aria-labelledby="quality-review-title">
      <header className="surface-header">
        <h2 id="quality-review-title">Quality & Review Center</h2>
        <p className="surface-note">
          Read-only, durable acceptance evidence. Gate outcomes are deterministic command results;
          review findings are structured reviewer evidence correlated to workflow execution.
        </p>
      </header>

      <form
        className="surface-controls"
        onSubmit={handleSubmit}
        aria-label="Filter quality and review evidence"
      >
        <label htmlFor="evidence-run-id">
          Workflow Run ID (Optional)
          <input
            id="evidence-run-id"
            data-testid="evidence-run-id-input"
            type="text"
            placeholder="Workflow UUID"
            value={runIdInput}
            onChange={(event) => setRunIdInput(event.target.value)}
          />
        </label>

        <label htmlFor="evidence-node-id">
          Node Run ID (Optional)
          <input
            id="evidence-node-id"
            data-testid="evidence-node-id-input"
            type="text"
            placeholder="Node UUID"
            value={nodeIdInput}
            onChange={(event) => setNodeIdInput(event.target.value)}
          />
        </label>

        <label htmlFor="evidence-limit">
          Limit per section
          <select
            id="evidence-limit"
            data-testid="evidence-limit-select"
            value={limit}
            onChange={(event) => setLimit(Number(event.target.value))}
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
          data-testid="evidence-apply-filter-button"
          disabled={loading}
        >
          {loading ? "Refreshing..." : "Filter & Refresh"}
        </button>
      </form>

      <section aria-labelledby="quality-gates-title">
        <h3 id="quality-gates-title">Quality Gate Results</h3>

        {qualityError && (
          <div role="alert" className="surface-alert" data-testid="quality-evidence-error">
            {qualityError}
          </div>
        )}

        {loading && <p data-testid="quality-evidence-loading">Loading quality-gate evidence...</p>}

        {!loading && !qualityError && qualityRecords.length === 0 && (
          <div className="surface-empty" data-testid="quality-evidence-empty">
            No quality-gate results found for the current query.
          </div>
        )}

        {qualityRecords.length > 0 && (
          <div className="surface-table-wrap">
            <table className="surface-table" aria-label="Persisted quality gate results">
              <caption>Deterministic Gate Evidence ({qualityRecords.length})</caption>
              <thead>
                <tr>
                  <th scope="col">Timestamp</th>
                  <th scope="col">Gate / Status</th>
                  <th scope="col">Correlation</th>
                  <th scope="col">Command</th>
                  <th scope="col">Exit / Duration</th>
                  <th scope="col">Evidence Log</th>
                </tr>
              </thead>
              <tbody>
                {qualityRecords.map((record, index) => {
                  const status = qualityStatus(record);
                  return (
                    <tr
                      key={`${record.workflow_run_id}-${record.node_run_id}-${record.created_at_millis}-${index}`}
                      data-testid={`quality-evidence-row-${index}`}
                    >
                      <td style={{ whiteSpace: "nowrap" }}>
                        {formatTimestamp(record.created_at_millis)}
                      </td>
                      <td>
                        <strong>{record.gate}</strong>
                        <div>
                          <span className={`surface-badge ${status.className}`}>{status.label}</span>
                        </div>
                      </td>
                      <td className="surface-mono">
                        <div title={record.workflow_run_id}>Run: {shortId(record.workflow_run_id)}</div>
                        <div title={record.node_run_id}>Node: {shortId(record.node_run_id)}</div>
                        {record.attempt_id && (
                          <div title={record.attempt_id}>Attempt: {shortId(record.attempt_id)}</div>
                        )}
                      </td>
                      <td className="surface-mono">{record.command || "—"}</td>
                      <td>
                        <div>Exit: {record.exit_code ?? "—"}</div>
                        <div>{formatDuration(record.duration_ms)}</div>
                      </td>
                      <td>
                        <details>
                          <summary>View log</summary>
                          <pre className="surface-log">{record.log_tail || "No output captured."}</pre>
                        </details>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <section aria-labelledby="review-findings-title">
        <h3 id="review-findings-title">Review Findings</h3>

        {reviewError && (
          <div role="alert" className="surface-alert" data-testid="review-findings-error">
            {reviewError}
          </div>
        )}

        {loading && <p data-testid="review-findings-loading">Loading review findings...</p>}

        {!loading && !reviewError && reviewFindings.length === 0 && (
          <div className="surface-empty" data-testid="review-findings-empty">
            No review findings found for the current query.
          </div>
        )}

        {reviewFindings.length > 0 && (
          <div className="surface-table-wrap">
            <table className="surface-table" aria-label="Persisted review findings">
              <caption>Structured Reviewer Findings ({reviewFindings.length})</caption>
              <thead>
                <tr>
                  <th scope="col">Timestamp</th>
                  <th scope="col">Severity</th>
                  <th scope="col">Correlation</th>
                  <th scope="col">Review Node</th>
                  <th scope="col">Location</th>
                  <th scope="col">Summary</th>
                  <th scope="col">Evidence</th>
                </tr>
              </thead>
              <tbody>
                {reviewFindings.map((finding, index) => (
                  <tr
                    key={`${finding.workflow_run_id}-${finding.node_run_id}-${finding.created_at_millis}-${index}`}
                    data-testid={`review-finding-row-${index}`}
                  >
                    <td style={{ whiteSpace: "nowrap" }}>
                      {formatTimestamp(finding.created_at_millis)}
                    </td>
                    <td>
                      <span className={`surface-badge ${severityClass(finding.severity)}`}>
                        {finding.severity}
                      </span>
                    </td>
                    <td className="surface-mono">
                      <div title={finding.workflow_run_id}>Run: {shortId(finding.workflow_run_id)}</div>
                      <div title={finding.node_run_id}>Node: {shortId(finding.node_run_id)}</div>
                      {finding.attempt_id && (
                        <div title={finding.attempt_id}>Attempt: {shortId(finding.attempt_id)}</div>
                      )}
                    </td>
                    <td className="surface-mono">{finding.node_key}</td>
                    <td className="surface-mono">
                      {finding.file}
                      {finding.line ? `:${finding.line}` : ""}
                    </td>
                    <td>{finding.summary}</td>
                    <td>
                      <details>
                        <summary>View evidence</summary>
                        <div className="surface-evidence-text">{finding.evidence}</div>
                      </details>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </section>
  );
}

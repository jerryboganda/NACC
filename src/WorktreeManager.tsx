import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  commands,
  type WorktreeLeaseView,
} from "./bindings";
import "./OperationalSurfaces.css";

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

function formatTimestamp(millis: string): string {
  if (!millis) return "—";
  const d = new Date(Number(millis));
  return Number.isNaN(d.getTime()) ? millis : d.toLocaleString();
}

function shortId(id: string): string {
  return id.length > 8 ? `${id.slice(0, 8)}…` : id;
}

function shortCommit(sha: string | null): string {
  if (!sha) return "—";
  return sha.length > 7 ? sha.slice(0, 7) : sha;
}

export default function WorktreeManager() {
  const [leases, setLeases] = useState<WorktreeLeaseView[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [projectIdInput, setProjectIdInput] = useState("");
  const [activeOnly, setActiveOnly] = useState(false);
  const [limit, setLimit] = useState(50);
  const mounted = useRef(false);
  const loadVersion = useRef(0);

  async function loadLeases() {
    const version = ++loadVersion.current;
    setLoading(true);
    setError(null);
    try {
      const clampedLimit = Math.max(1, Math.min(500, Number(limit) || 50));
      const cleanProjectId = projectIdInput.trim() || null;
      const result = await commands.listWorktreeLeases({
        project_id: cleanProjectId,
        active_only: activeOnly,
        limit: clampedLimit,
      });

      if (!mounted.current || version !== loadVersion.current) return;
      if (result.status === "error") throw new Error(result.error);
      setLeases(result.data);
    } catch (err) {
      if (mounted.current && version === loadVersion.current) {
        setError(`Failed to retrieve worktree leases: ${message(err)}`);
      }
    } finally {
      if (mounted.current && version === loadVersion.current) {
        setLoading(false);
      }
    }
  }

  useEffect(() => {
    mounted.current = true;
    void loadLeases();
    return () => {
      mounted.current = false;
    };
  }, []);

  function handleSubmit(e: FormEvent) {
    e.preventDefault();
    void loadLeases();
  }

  return (
    <section className="operational-surface" aria-labelledby="worktree-manager-title">
      <header className="surface-header">
        <h2 id="worktree-manager-title">Worktree Manager</h2>
        <p className="surface-note">
          Read-only observability over Git worktree leases. Allocation, release, and quarantine
          lifecycle mutations are strictly managed by the workflow engine; destructive manual
          deletion is prohibited to protect uncommitted agent work.
        </p>
      </header>

      {error && (
        <div role="alert" className="surface-alert" data-testid="worktree-error">
          {error}
        </div>
      )}

      <form className="surface-controls" onSubmit={handleSubmit} aria-label="Filter worktree leases">
        <label htmlFor="worktree-project-id">
          Project ID (Optional)
          <input
            id="worktree-project-id"
            data-testid="worktree-project-id-input"
            type="text"
            placeholder="UUID (e.g. 5b6f2b2e...)"
            value={projectIdInput}
            onChange={(e) => setProjectIdInput(e.target.value)}
          />
        </label>

        <label style={{ flexDirection: "row", alignItems: "center", minHeight: "40px" }}>
          <input
            type="checkbox"
            data-testid="worktree-active-only-toggle"
            checked={activeOnly}
            onChange={(e) => setActiveOnly(e.target.checked)}
          />
          Active Leases Only
        </label>

        <label htmlFor="worktree-limit">
          Limit (1–500)
          <input
            id="worktree-limit"
            data-testid="worktree-limit-input"
            type="number"
            min={1}
            max={500}
            value={limit}
            onChange={(e) => setLimit(Number(e.target.value))}
          />
        </label>

        <button
          type="submit"
          data-testid="worktree-apply-filter-button"
          disabled={loading}
        >
          {loading ? "Refreshing..." : "Apply & Refresh"}
        </button>
      </form>

      {loading && !leases.length && (
        <p data-testid="worktree-loading">Loading worktree leases...</p>
      )}

      {!loading && leases.length === 0 && (
        <div className="surface-empty" data-testid="worktree-empty">
          No worktree leases match the selected criteria.
        </div>
      )}

      {leases.length > 0 && (
        <div className="surface-table-wrap">
          <table className="surface-table" aria-label="Worktree lease records">
            <caption>
              Recorded Worktree Leases ({leases.length})
            </caption>
            <thead>
              <tr>
                <th scope="col">Lease ID</th>
                <th scope="col">State</th>
                <th scope="col">Project / Run</th>
                <th scope="col">Branch & Path</th>
                <th scope="col">Base Commit</th>
                <th scope="col">Quarantine Reason</th>
                <th scope="col">Created</th>
              </tr>
            </thead>
            <tbody>
              {leases.map((lease) => (
                <tr key={lease.id} data-testid={`worktree-row-${lease.id}`}>
                  <td className="surface-mono" title={lease.id}>
                    {shortId(lease.id)}
                  </td>
                  <td>
                    <span
                      className={`surface-badge badge-${lease.state}`}
                      data-testid={`lease-state-${lease.id}`}
                    >
                      {lease.state}
                    </span>
                  </td>
                  <td className="surface-mono">
                    <span title={lease.project_id}>Proj: {shortId(lease.project_id)}</span>
                    {lease.workflow_run_id && (
                      <div style={{ opacity: 0.75 }} title={lease.workflow_run_id}>
                        Run: {shortId(lease.workflow_run_id)}
                      </div>
                    )}
                  </td>
                  <td>
                    <strong>{lease.branch}</strong>
                    <div className="surface-mono" style={{ fontSize: "0.8rem", opacity: 0.75 }}>
                      {lease.path}
                    </div>
                  </td>
                  <td className="surface-mono">
                    <span title={lease.base_commit}>{shortCommit(lease.base_commit)}</span>
                    {lease.head_commit && (
                      <div style={{ opacity: 0.75 }} title={lease.head_commit}>
                        Head: {shortCommit(lease.head_commit)}
                      </div>
                    )}
                  </td>
                  <td>
                    {lease.quarantine_reason ? (
                      <span style={{ color: "#ef4444" }}>{lease.quarantine_reason}</span>
                    ) : (
                      <span style={{ opacity: 0.5 }}>—</span>
                    )}
                  </td>
                  <td>{formatTimestamp(lease.created_at_millis)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  commands,
  type ArtifactSummary,
  type BranchSummary,
  type CheckRunSummary,
  type EnvironmentSummary,
  type FailedRunEvidence,
  type GithubAuthStatus,
  type PendingDeploymentSummary,
  type PullRequestSummary,
  type RepositorySummary,
  type WorkflowJobSummary,
  type WorkflowRunSummary,
} from "./bindings";
import "./CICDCenter.css";

const LIST_LIMIT = 30;

const message = (error: unknown) => error instanceof Error ? error.message : String(error);

function displayDate(value: string | null) {
  if (!value) return "—";
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString();
}

function shortSha(sha: string) {
  return sha.length > 12 ? sha.slice(0, 12) : sha;
}

export default function CICDCenter() {
  const [auth, setAuth] = useState<GithubAuthStatus | null>(null);
  const [authLoading, setAuthLoading] = useState(true);
  const [repositoryInput, setRepositoryInput] = useState("");
  const [repository, setRepository] = useState<RepositorySummary | null>(null);
  const [branches, setBranches] = useState<BranchSummary[]>([]);
  const [pullRequests, setPullRequests] = useState<PullRequestSummary[]>([]);
  const [runs, setRuns] = useState<WorkflowRunSummary[]>([]);
  const [artifacts, setArtifacts] = useState<ArtifactSummary[]>([]);
  const [environments, setEnvironments] = useState<EnvironmentSummary[]>([]);
  const [selectedRun, setSelectedRun] = useState<WorkflowRunSummary | null>(null);
  const [jobs, setJobs] = useState<WorkflowJobSummary[]>([]);
  const [checks, setChecks] = useState<CheckRunSummary[]>([]);
  const [evidence, setEvidence] = useState<FailedRunEvidence[]>([]);
  const [pendingDeployments, setPendingDeployments] = useState<PendingDeploymentSummary[]>([]);
  const [repoLoading, setRepoLoading] = useState(false);
  const [detailLoading, setDetailLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState("");
  const [approvedBy, setApprovedBy] = useState("");
  const [rerunReason, setRerunReason] = useState("");
  const mounted = useRef(false);
  const repositoryVersion = useRef(0);
  const detailVersion = useRef(0);

  useEffect(() => {
    mounted.current = true;
    let cancelled = false;
    commands.getGithubAuthStatus()
      .then((result) => {
        if (cancelled) return;
        if (result.status === "ok") setAuth(result.data);
        else setError(`GitHub authentication check failed: ${result.error}`);
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(`GitHub authentication check failed: ${message(err)}`);
      })
      .finally(() => {
        if (!cancelled) setAuthLoading(false);
      });
    return () => {
      cancelled = true;
      mounted.current = false;
    };
  }, []);

  function clearRunDetail() {
    detailVersion.current += 1;
    setSelectedRun(null);
    setJobs([]);
    setChecks([]);
    setEvidence([]);
    setPendingDeployments([]);
    setApprovedBy("");
    setRerunReason("");
  }

  async function loadRepositoryData(slug: string) {
    const normalized = slug.trim();
    if (!normalized) {
      setError("Enter a GitHub repository as owner/name.");
      return;
    }
    if (auth && !auth.authenticated) {
      setError("GitHub CLI is not authenticated. Sign in with gh before loading repository data.");
      return;
    }

    const version = ++repositoryVersion.current;
    setRepoLoading(true);
    setError(null);
    setNotice("");
    clearRunDetail();
    try {
      const repoResult = await commands.getGithubRepository({ repository: normalized });
      if (!mounted.current || version !== repositoryVersion.current) return;
      if (repoResult.status === "error") throw new Error(repoResult.error);
      const canonical = repoResult.data.name_with_owner;
      const [branchesResult, prsResult, runsResult, artifactsResult, environmentsResult] = await Promise.all([
        commands.listGithubBranches({ repository: canonical, limit: LIST_LIMIT }),
        commands.listGithubPullRequests({ repository: canonical, limit: LIST_LIMIT }),
        commands.listGithubWorkflowRuns({ repository: canonical, limit: LIST_LIMIT }),
        commands.listGithubArtifacts({ repository: canonical, limit: LIST_LIMIT }),
        commands.listGithubEnvironments({ repository: canonical, limit: LIST_LIMIT }),
      ]);
      if (!mounted.current || version !== repositoryVersion.current) return;

      setRepository(repoResult.data);
      setRepositoryInput(canonical);
      const partialErrors: string[] = [];
      if (branchesResult.status === "ok") setBranches(branchesResult.data);
      else { setBranches([]); partialErrors.push(`branches: ${branchesResult.error}`); }
      if (prsResult.status === "ok") setPullRequests(prsResult.data);
      else { setPullRequests([]); partialErrors.push(`pull requests: ${prsResult.error}`); }
      if (runsResult.status === "ok") setRuns(runsResult.data);
      else { setRuns([]); partialErrors.push(`workflow runs: ${runsResult.error}`); }
      if (artifactsResult.status === "ok") setArtifacts(artifactsResult.data);
      else { setArtifacts([]); partialErrors.push(`artifacts: ${artifactsResult.error}`); }
      if (environmentsResult.status === "ok") setEnvironments(environmentsResult.data);
      else { setEnvironments([]); partialErrors.push(`environments: ${environmentsResult.error}`); }

      if (partialErrors.length) {
        setError(`Repository loaded with partial GitHub data. ${partialErrors.join(" | ")}`);
      } else {
        setNotice(`Loaded ${canonical} from the authenticated GitHub CLI.`);
      }
    } catch (err) {
      if (mounted.current && version === repositoryVersion.current) setError(message(err));
    } finally {
      if (mounted.current && version === repositoryVersion.current) setRepoLoading(false);
    }
  }

  function submitRepository(event: FormEvent) {
    event.preventDefault();
    void loadRepositoryData(repositoryInput);
  }

  async function openRun(run: WorkflowRunSummary) {
    if (!repository) return;
    const version = ++detailVersion.current;
    setSelectedRun(run);
    setDetailLoading(true);
    setError(null);
    setNotice("");
    setJobs([]);
    setChecks([]);
    setEvidence([]);
    setPendingDeployments([]);
    try {
      const [jobsResult, checksResult, evidenceResult, deploymentsResult] = await Promise.all([
        commands.listGithubWorkflowJobs({ repository: repository.name_with_owner, run_id: run.database_id }),
        commands.listGithubCheckRuns({
          repository: repository.name_with_owner,
          commit_sha: run.head_sha,
          limit: LIST_LIMIT,
        }),
        commands.getGithubFailedRunEvidence({ repository: repository.name_with_owner, run_id: run.database_id }),
        commands.listGithubPendingDeployments({ repository: repository.name_with_owner, run_id: run.database_id }),
      ]);
      if (!mounted.current || version !== detailVersion.current) return;

      const partialErrors: string[] = [];
      if (jobsResult.status === "ok") setJobs(jobsResult.data);
      else partialErrors.push(`jobs: ${jobsResult.error}`);
      if (checksResult.status === "ok") setChecks(checksResult.data);
      else partialErrors.push(`checks: ${checksResult.error}`);
      if (evidenceResult.status === "ok") setEvidence(evidenceResult.data);
      else partialErrors.push(`failed-run evidence: ${evidenceResult.error}`);
      if (deploymentsResult.status === "ok") setPendingDeployments(deploymentsResult.data);
      else partialErrors.push(`pending deployments: ${deploymentsResult.error}`);
      if (partialErrors.length) setError(`Run loaded with partial GitHub data. ${partialErrors.join(" | ")}`);
    } catch (err) {
      if (mounted.current && version === detailVersion.current) setError(message(err));
    } finally {
      if (mounted.current && version === detailVersion.current) setDetailLoading(false);
    }
  }

  function rerunFailed() {
    if (!repository || !selectedRun) return;
    if (selectedRun.conclusion !== "failure") {
      setError("Only a failed workflow run can use the failed-jobs rerun action.");
      return;
    }
    const actor = approvedBy.trim();
    const reason = rerunReason.trim();
    if (!actor || !reason) {
      setError("Rerunning failed jobs requires both the human approver and an explicit reason.");
      return;
    }
    if (busy) return;
    setBusy(true);
    setError(null);
    setNotice("");
    void commands.rerunFailedGithubWorkflow({
      repository: repository.name_with_owner,
      run_id: selectedRun.database_id,
      approved_by: actor,
      reason,
    }).then(async (result) => {
      if (!mounted.current) return;
      if (result.status === "error") throw new Error(result.error);
      await loadRepositoryData(repository.name_with_owner);
      if (!mounted.current) return;
      setNotice(`Failed jobs for workflow run ${selectedRun.database_id} were rerun with explicit approval.`);
    }).catch((err: unknown) => {
      if (mounted.current) setError(message(err));
    }).finally(() => {
      if (mounted.current) setBusy(false);
    });
  }

  return (
    <section className="cicd-center" aria-labelledby="cicd-center-title">
      <div className="cicd-heading">
        <div>
          <h2 id="cicd-center-title">CI/CD Center</h2>
          <p>Live GitHub and GitHub Actions state through your authenticated <code>gh</code> CLI. NACC never copies the GitHub token.</p>
        </div>
        <div className="cicd-auth" aria-live="polite">
          {authLoading && <span>Checking GitHub authentication…</span>}
          {!authLoading && auth?.authenticated && <span>Authenticated as <strong>{auth.login ?? "GitHub user"}</strong></span>}
          {!authLoading && auth && !auth.authenticated && <span>GitHub CLI is not authenticated.</span>}
        </div>
      </div>

      {error && <p role="alert" className="cicd-message">{error}</p>}
      {notice && <p role="status" className="cicd-message">{notice}</p>}

      <form className="cicd-repository-form" onSubmit={submitRepository}>
        <label>
          Repository
          <input
            value={repositoryInput}
            onChange={(event) => setRepositoryInput(event.target.value)}
            placeholder="owner/repository"
            autoCapitalize="none"
            autoCorrect="off"
          />
        </label>
        <button type="submit" disabled={repoLoading || authLoading || auth?.authenticated === false}>
          {repoLoading ? "Loading…" : "Load repository"}
        </button>
        {repository && (
          <button type="button" disabled={repoLoading} onClick={() => void loadRepositoryData(repository.name_with_owner)}>
            Refresh repository
          </button>
        )}
      </form>

      {repository && (
        <>
          <div className="cicd-summary" data-testid="github-repository-summary">
            <div><span>Repository</span><strong>{repository.name_with_owner}</strong></div>
            <div><span>Default branch</span><strong>{repository.default_branch ?? "unknown"}</strong></div>
            <div><span>Visibility</span><strong>{repository.is_private ? "private" : "public"}</strong></div>
            <div><span>Loaded records</span><strong>{runs.length} runs · {pullRequests.length} PRs · {artifacts.length} artifacts</strong></div>
          </div>

          <div className="cicd-grid">
            <section aria-labelledby="github-runs-title">
              <h3 id="github-runs-title">Workflow runs</h3>
              {runs.length === 0 ? <p>No workflow runs returned.</p> : (
                <div className="cicd-table-wrap">
                  <table>
                    <caption>Recent GitHub Actions workflow runs</caption>
                    <thead><tr><th>Workflow</th><th>Status</th><th>Branch</th><th>Updated</th><th>Action</th></tr></thead>
                    <tbody>
                      {runs.map((run) => (
                        <tr key={run.database_id}>
                          <td>{run.workflow_name || run.name}<small><code>{shortSha(run.head_sha)}</code> · {run.event}</small></td>
                          <td>{run.conclusion ?? run.status}</td>
                          <td>{run.head_branch ?? "—"}</td>
                          <td>{displayDate(run.updated_at)}</td>
                          <td><button type="button" onClick={() => void openRun(run)}>Inspect run {run.database_id}</button></td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>

            <section aria-labelledby="github-prs-title">
              <h3 id="github-prs-title">Pull requests</h3>
              {pullRequests.length === 0 ? <p>No open pull requests returned.</p> : (
                <div className="cicd-table-wrap">
                  <table>
                    <caption>Recent pull requests</caption>
                    <thead><tr><th>PR</th><th>State</th><th>Branch</th><th>Author</th></tr></thead>
                    <tbody>{pullRequests.map((pr) => (
                      <tr key={pr.number}>
                        <td>#{pr.number} {pr.title}{pr.is_draft ? " (draft)" : ""}</td>
                        <td>{pr.merge_state_status ?? pr.state}</td>
                        <td>{pr.head_ref_name} → {pr.base_ref_name}</td>
                        <td>{pr.author_login ?? "—"}</td>
                      </tr>
                    ))}</tbody>
                  </table>
                </div>
              )}
            </section>
          </div>

          <details>
            <summary>Repository branches, artifacts, and environments</summary>
            <div className="cicd-grid cicd-secondary-grid">
              <section>
                <h3>Branches</h3>
                {branches.length === 0 ? <p>No branches returned.</p> : <ul className="cicd-list">{branches.map((branch) => (
                  <li key={branch.name}><strong>{branch.name}</strong><span><code>{shortSha(branch.commit_sha)}</code>{branch.protected ? " · protected" : ""}</span></li>
                ))}</ul>}
              </section>
              <section>
                <h3>Artifacts</h3>
                {artifacts.length === 0 ? <p>No artifacts returned.</p> : <ul className="cicd-list">{artifacts.map((artifact) => (
                  <li key={artifact.id}><strong>{artifact.name}</strong><span>{artifact.size_in_bytes} B · {artifact.expired ? "expired" : `expires ${displayDate(artifact.expires_at)}`}</span></li>
                ))}</ul>}
              </section>
              <section>
                <h3>Environments</h3>
                {environments.length === 0 ? <p>No environments returned.</p> : <ul className="cicd-list">{environments.map((environment) => (
                  <li key={environment.id}><strong>{environment.name}</strong><span>{environment.protection_rule_count} protection rule(s){environment.can_admins_bypass == null ? "" : environment.can_admins_bypass ? " · admin bypass allowed" : " · admin bypass blocked"}</span></li>
                ))}</ul>}
              </section>
            </div>
          </details>

          {selectedRun && (
            <section className="cicd-run-detail" aria-labelledby="github-run-detail-title">
              <div className="cicd-run-title">
                <div>
                  <h3 id="github-run-detail-title">Run {selectedRun.database_id}: {selectedRun.workflow_name || selectedRun.name}</h3>
                  <p>{selectedRun.conclusion ?? selectedRun.status} · <code>{selectedRun.head_sha}</code></p>
                </div>
                <button type="button" disabled={detailLoading} onClick={() => void openRun(selectedRun)}>
                  {detailLoading ? "Loading run…" : "Refresh run"}
                </button>
              </div>

              <div className="cicd-grid">
                <section>
                  <h4>Jobs and steps</h4>
                  {detailLoading && jobs.length === 0 ? <p>Loading jobs…</p> : jobs.length === 0 ? <p>No jobs returned.</p> : (
                    <ul className="cicd-job-list">{jobs.map((job) => (
                      <li key={job.database_id}>
                        <strong>{job.name}</strong> <span>{job.conclusion ?? job.status}</span>
                        {job.steps.length > 0 && <ol>{job.steps.map((step) => (
                          <li key={`${job.database_id}-${step.number}`}>{step.number}. {step.name} — {step.conclusion ?? step.status}</li>
                        ))}</ol>}
                      </li>
                    ))}</ul>
                  )}
                </section>
                <section>
                  <h4>Checks</h4>
                  {detailLoading && checks.length === 0 ? <p>Loading checks…</p> : checks.length === 0 ? <p>No check runs returned for this commit.</p> : (
                    <ul className="cicd-list">{checks.map((check) => (
                      <li key={check.id}><strong>{check.name}</strong><span>{check.conclusion ?? check.status}{check.app_name ? ` · ${check.app_name}` : ""}</span></li>
                    ))}</ul>
                  )}
                </section>
              </div>

              <section aria-labelledby="github-failure-evidence-title">
                <h4 id="github-failure-evidence-title">Failure evidence</h4>
                {evidence.length === 0 ? <p>No failed-job evidence returned.</p> : evidence.map((item) => (
                  <article className="cicd-evidence" key={item.job_id}>
                    <h5>{item.job_name}: {item.classification.replaceAll("_", " ")}</h5>
                    <p>{item.matched_evidence ? `Matched evidence: ${item.matched_evidence}` : "No deterministic classifier rule matched this failure."}</p>
                    <pre>{item.log_excerpt}</pre>
                  </article>
                ))}
              </section>

              <section aria-labelledby="github-deployments-title">
                <h4 id="github-deployments-title">Pending deployments</h4>
                {pendingDeployments.length === 0 ? <p>No pending deployment approvals returned.</p> : (
                  <ul className="cicd-list">{pendingDeployments.map((deployment) => (
                    <li key={deployment.environment_id}>
                      <strong>{deployment.environment_name}</strong>
                      <span>{deployment.current_user_can_approve ? "Current GitHub user may approve" : "Current GitHub user cannot approve"} · wait timer {deployment.wait_timer}s{deployment.reviewer_logins.length ? ` · reviewers: ${deployment.reviewer_logins.join(", ")}` : ""}</span>
                    </li>
                  ))}</ul>
                )}
                <p><small>Deployment approval is read-only here until a separately approval-gated backend mutation is implemented.</small></p>
              </section>

              {selectedRun.conclusion === "failure" && (
                <fieldset className="cicd-rerun">
                  <legend>Rerun failed jobs</legend>
                  <p>This mutation is never automatic. It requires an identified human approver and a reason on every invocation.</p>
                  <label>
                    Approved by
                    <input value={approvedBy} onChange={(event) => setApprovedBy(event.target.value)} disabled={busy} />
                  </label>
                  <label>
                    Approval reason
                    <textarea value={rerunReason} onChange={(event) => setRerunReason(event.target.value)} disabled={busy} rows={3} />
                  </label>
                  <button type="button" onClick={rerunFailed} disabled={busy}>{busy ? "Rerunning…" : "Rerun failed jobs"}</button>
                </fieldset>
              )}
            </section>
          )}
        </>
      )}
    </section>
  );
}

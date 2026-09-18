import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  CheckRunSummary,
  FailedRunEvidence,
  PendingDeploymentSummary,
  RepositorySummary,
  WorkflowJobSummary,
  WorkflowRunSummary,
} from "./bindings";

const mocks = vi.hoisted(() => ({
  getGithubAuthStatus: vi.fn(),
  getGithubRepository: vi.fn(),
  listGithubBranches: vi.fn(),
  listGithubPullRequests: vi.fn(),
  listGithubCheckRuns: vi.fn(),
  listGithubWorkflowRuns: vi.fn(),
  listGithubWorkflowJobs: vi.fn(),
  getGithubFailedRunEvidence: vi.fn(),
  listGithubArtifacts: vi.fn(),
  listGithubEnvironments: vi.fn(),
  listGithubPendingDeployments: vi.fn(),
  rerunFailedGithubWorkflow: vi.fn(),
}));

vi.mock("./bindings", () => ({ commands: mocks }));
import CICDCenter from "./CICDCenter";

const ok = <T,>(data: T) => ({ status: "ok" as const, data });

const repository: RepositorySummary = {
  name_with_owner: "owner/repo",
  default_branch: "main",
  is_private: false,
  url: "https://github.com/owner/repo",
};

const failedRun: WorkflowRunSummary = {
  database_id: "12345678901234567890",
  name: "CI",
  workflow_name: "CI",
  status: "completed",
  conclusion: "failure",
  event: "push",
  head_branch: "main",
  head_sha: "0123456789abcdef0123456789abcdef01234567",
  url: "https://github.com/owner/repo/actions/runs/123",
  created_at: "2026-09-18T12:00:00Z",
  updated_at: "2026-09-18T12:05:00Z",
};

const job: WorkflowJobSummary = {
  database_id: "9007199254740993",
  name: "Build",
  status: "completed",
  conclusion: "failure",
  started_at: "2026-09-18T12:00:00Z",
  completed_at: "2026-09-18T12:04:00Z",
  url: "https://github.com/owner/repo/actions/runs/123/job/456",
  steps: [{
    number: 1,
    name: "cargo test",
    status: "completed",
    conclusion: "failure",
    started_at: "2026-09-18T12:01:00Z",
    completed_at: "2026-09-18T12:03:00Z",
  }],
};

const check: CheckRunSummary = {
  id: "9007199254740995",
  name: "Rust / GNU",
  status: "completed",
  conclusion: "failure",
  details_url: null,
  started_at: null,
  completed_at: null,
  app_name: "GitHub Actions",
};

const failure: FailedRunEvidence = {
  run_id: failedRun.database_id,
  job_id: job.database_id,
  job_name: "Build",
  classification: "product_defect",
  matched_evidence: "assertion",
  log_excerpt: "assertion failed: expected green build",
};

const deployment: PendingDeploymentSummary = {
  environment_id: "9007199254740997",
  environment_name: "production",
  wait_timer: 0,
  current_user_can_approve: false,
  reviewer_logins: ["release-admin"],
};

beforeEach(() => {
  vi.resetAllMocks();
  mocks.getGithubAuthStatus.mockResolvedValue(ok({ authenticated: true, login: "operator" }));
  mocks.getGithubRepository.mockResolvedValue(ok(repository));
  mocks.listGithubBranches.mockResolvedValue(ok([]));
  mocks.listGithubPullRequests.mockResolvedValue(ok([]));
  mocks.listGithubWorkflowRuns.mockResolvedValue(ok([]));
  mocks.listGithubArtifacts.mockResolvedValue(ok([]));
  mocks.listGithubEnvironments.mockResolvedValue(ok([]));
  mocks.listGithubWorkflowJobs.mockResolvedValue(ok([]));
  mocks.listGithubCheckRuns.mockResolvedValue(ok([]));
  mocks.getGithubFailedRunEvidence.mockResolvedValue(ok([]));
  mocks.listGithubPendingDeployments.mockResolvedValue(ok([]));
  mocks.rerunFailedGithubWorkflow.mockResolvedValue(ok(null));
});

async function loadRepository() {
  await screen.findByText(/Authenticated as/);
  fireEvent.change(screen.getByLabelText("Repository"), { target: { value: "owner/repo" } });
  fireEvent.click(screen.getByRole("button", { name: "Load repository" }));
  await screen.findByTestId("github-repository-summary");
}

describe("CI/CD Center", () => {
  it("loads authenticated repository state through typed GitHub commands", async () => {
    mocks.listGithubWorkflowRuns.mockResolvedValue(ok([failedRun]));
    render(<CICDCenter />);
    await loadRepository();

    expect(screen.getByText("owner/repo")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: `Inspect run ${failedRun.database_id}` })).toBeInTheDocument();
    expect(mocks.getGithubRepository).toHaveBeenCalledExactlyOnceWith({ repository: "owner/repo" });
    expect(mocks.listGithubWorkflowRuns).toHaveBeenCalledExactlyOnceWith({ repository: "owner/repo", limit: 30 });
    expect(mocks.listGithubArtifacts).toHaveBeenCalledExactlyOnceWith({ repository: "owner/repo", limit: 30 });
  });

  it("shows jobs, checks, deterministic failure evidence, and pending deployment state", async () => {
    mocks.listGithubWorkflowRuns.mockResolvedValue(ok([failedRun]));
    mocks.listGithubWorkflowJobs.mockResolvedValue(ok([job]));
    mocks.listGithubCheckRuns.mockResolvedValue(ok([check]));
    mocks.getGithubFailedRunEvidence.mockResolvedValue(ok([failure]));
    mocks.listGithubPendingDeployments.mockResolvedValue(ok([deployment]));
    render(<CICDCenter />);
    await loadRepository();

    fireEvent.click(screen.getByRole("button", { name: `Inspect run ${failedRun.database_id}` }));
    expect(await screen.findByText("cargo test", { exact: false })).toBeInTheDocument();
    expect(screen.getByText("Rust / GNU")).toBeInTheDocument();
    expect(screen.getByText(/Build: product defect/)).toBeInTheDocument();
    expect(screen.getByText(/assertion failed: expected green build/)).toBeInTheDocument();
    expect(screen.getByText("production")).toBeInTheDocument();
    expect(mocks.listGithubWorkflowJobs).toHaveBeenCalledExactlyOnceWith({
      repository: "owner/repo",
      run_id: failedRun.database_id,
    });
  });

  it("requires explicit human approval before rerunning failed jobs", async () => {
    mocks.listGithubWorkflowRuns.mockResolvedValue(ok([failedRun]));
    render(<CICDCenter />);
    await loadRepository();
    fireEvent.click(screen.getByRole("button", { name: `Inspect run ${failedRun.database_id}` }));
    await screen.findByRole("button", { name: "Rerun failed jobs" });

    fireEvent.click(screen.getByRole("button", { name: "Rerun failed jobs" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/human approver/i);
    expect(mocks.rerunFailedGithubWorkflow).not.toHaveBeenCalled();

    fireEvent.change(screen.getByLabelText("Approved by"), { target: { value: "Dr Release" } });
    fireEvent.change(screen.getByLabelText("Approval reason"), { target: { value: "Failure classified and reviewed" } });
    fireEvent.click(screen.getByRole("button", { name: "Rerun failed jobs" }));

    await waitFor(() => expect(mocks.rerunFailedGithubWorkflow).toHaveBeenCalledExactlyOnceWith({
      repository: "owner/repo",
      run_id: failedRun.database_id,
      approved_by: "Dr Release",
      reason: "Failure classified and reviewed",
    }));
  });

  it("keeps successful repository data visible when one secondary GitHub query fails", async () => {
    mocks.listGithubEnvironments.mockResolvedValue({ status: "error", error: "environments forbidden" });
    render(<CICDCenter />);
    await loadRepository();

    expect(screen.getByTestId("github-repository-summary")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent(/partial GitHub data/i);
    expect(screen.getByRole("alert")).toHaveTextContent(/environments forbidden/i);
  });
});

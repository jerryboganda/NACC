import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ApprovalView,
  NodeRunView,
  RunSnapshotView,
  WorkflowEventView,
  WorkflowRunView,
  WorkflowTemplateView,
} from "./bindings";

// Same IPC-boundary mock style as RoleMatrix.test.tsx: the generated
// bindings module is replaced wholesale, so these tests verify the panel's
// own behavior against realistic view fixtures.
const mocks = vi.hoisted(() => ({
  listWorkflowTemplates: vi.fn(), startWorkflowRun: vi.fn(), getWorkflowRun: vi.fn(),
  listWorkflowRuns: vi.fn(), pauseWorkflowRun: vi.fn(), cancelWorkflowRun: vi.fn(),
  resumeWorkflowRun: vi.fn(), decideWorkflowApproval: vi.fn(), listWorkflowEvents: vi.fn(),
}));
vi.mock("./bindings", () => ({ commands: mocks }));
import RunConsole from "./RunConsole";

const template: WorkflowTemplateView = {
  name: "Explore then plan", description: "Reconnaissance followed by a plan.", node_keys: ["explore", "plan"],
  version: 1, is_built_in: true,
};
const run: WorkflowRunView = {
  id: "run-1", project_id: "proj-1", template_name: "Explore then plan", state: "running",
  note: null, created_at_millis: "1", updated_at_millis: "2",
};
const node: NodeRunView = {
  id: "node-1", node_key: "explore", title: "Explore the repository", state: "succeeded", attempts: 1,
  requires_approval: false, last_detail: "found the entry point",
};
const approval: ApprovalView = {
  id: "appr-1", node_run_id: "node-2", node_key: "explore", summary: "Findings ready to hand off",
  decision: null, decided_at_millis: null,
};
const checkpoint = { sequence: 1, state: "running" as const, detail: "node explore succeeded", created_at_millis: "3" };
const event: WorkflowEventView = { event_type: "session_started", payload_json: '{"session":"s-1"}', created_at_millis: "4" };
const snapshot: RunSnapshotView = {
  run, nodes: [node], approvals: [approval], checkpoints: [checkpoint],
  pending_approval_count: 1, finished: false,
};
const ok = <T,>(data: T) => ({ status: "ok", data });

beforeEach(() => {
  vi.resetAllMocks();
  mocks.listWorkflowTemplates.mockResolvedValue(ok([template]));
  mocks.listWorkflowRuns.mockResolvedValue(ok([]));
  mocks.getWorkflowRun.mockResolvedValue(ok(snapshot));
  mocks.listWorkflowEvents.mockResolvedValue(ok([event]));
});

describe("Run Console", () => {
  it("lists the built-in templates and the durable runs", async () => {
    mocks.listWorkflowRuns.mockResolvedValue(ok([]));
    render(<RunConsole />);
    expect(screen.getByText("Loading workflows…")).toBeInTheDocument();
    expect(await screen.findByRole("option", { name: "Explore then plan" })).toBeInTheDocument();
    expect(screen.getByText("No workflow runs yet.")).toBeInTheDocument();
    expect(screen.getByText(/Nodes: explore, plan/)).toBeInTheDocument();
  });

  it("reports load errors accessibly and keeps the refresh control", async () => {
    mocks.listWorkflowRuns.mockRejectedValueOnce(new Error("IPC unavailable"));
    render(<RunConsole />);
    expect(await screen.findByRole("alert")).toHaveTextContent("IPC unavailable");
    mocks.listWorkflowRuns.mockResolvedValue(ok([]));
    fireEvent.click(screen.getByRole("button", { name: "Refresh runs" }));
    await screen.findByText("No workflow runs yet.");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("starts a run through the typed command with an explicit workspace", async () => {
    mocks.startWorkflowRun.mockResolvedValue(ok(snapshot));
    render(<RunConsole />);
    await screen.findByRole("option", { name: "Explore then plan" });
    fireEvent.change(screen.getByLabelText("Project ID"), { target: { value: "proj-1" } });
    fireEvent.change(screen.getByLabelText("Workspace (absolute path)"), { target: { value: "D:\\repo" } });
    fireEvent.change(
      screen.getByLabelText(/Worktrees root \(optional/),
      { target: { value: "D:\\repo-worktrees" } },
    );
    fireEvent.click(screen.getByRole("button", { name: "Start run" }));
    await screen.findByTestId("run-state");
    expect(mocks.startWorkflowRun).toHaveBeenCalledExactlyOnceWith({
      project_id: "proj-1", template_name: "Explore then plan", workspace: "D:\\repo",
      worktrees_root: "D:\\repo-worktrees",
    });
    expect(mocks.getWorkflowRun).toHaveBeenCalledWith({ run_id: "run-1" });
  });

  it("refuses to start without an explicit workspace and never calls IPC", async () => {
    render(<RunConsole />);
    await screen.findByRole("option", { name: "Explore then plan" });
    fireEvent.change(screen.getByLabelText("Project ID"), { target: { value: "proj-1" } });
    const form = screen.getByRole("button", { name: "Start run" }).closest("form")!;
    fireEvent.submit(form);
    expect(await screen.findByRole("alert")).toHaveTextContent(/workspace/i);
    expect(mocks.startWorkflowRun).not.toHaveBeenCalled();
  });

  it("surfaces a backend refusal without losing the typed workspace", async () => {
    mocks.startWorkflowRun.mockResolvedValue({
      status: "error", error: "workspace is not an existing directory: D:\\nope",
    });
    render(<RunConsole />);
    await screen.findByRole("option", { name: "Explore then plan" });
    fireEvent.change(screen.getByLabelText("Project ID"), { target: { value: "proj-1" } });
    fireEvent.change(screen.getByLabelText("Workspace (absolute path)"), { target: { value: "D:\\nope" } });
    fireEvent.click(screen.getByRole("button", { name: "Start run" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("workspace is not an existing directory: D:\\nope");
    expect(screen.getByLabelText("Workspace (absolute path)")).toHaveValue("D:\\nope");
  });

  it("opens a run and shows nodes, checkpoints, and the verbatim event payload", async () => {
    mocks.listWorkflowRuns.mockResolvedValue(ok([run]));
    render(<RunConsole />);
    fireEvent.click(await screen.findByRole("button", { name: "Open run run-1" }));
    await screen.findByTestId("run-state");
    expect(mocks.getWorkflowRun).toHaveBeenCalledExactlyOnceWith({ run_id: "run-1" });
    expect(screen.getByText("Explore the repository")).toBeInTheDocument();
    expect(screen.getByText(/node explore succeeded/)).toBeInTheDocument();
    expect(screen.getByText("session_started")).toBeInTheDocument();
    expect(screen.getByText('{"session":"s-1"}')).toBeInTheDocument();
  });

  it("ignores a stale detail response for a run that is no longer selected", async () => {
    const runB: WorkflowRunView = { ...run, id: "run-2", state: "running" };
    const snapshotB: RunSnapshotView = { ...snapshot, run: runB };
    let resolveFirst!: (value: ReturnType<typeof ok<RunSnapshotView>>) => void;
    mocks.getWorkflowRun.mockImplementation(({ run_id }: { run_id: string }) =>
      run_id === "run-1"
        ? new Promise(resolve => { resolveFirst = resolve; })
        : Promise.resolve(ok(snapshotB)));
    mocks.listWorkflowRuns.mockResolvedValue(ok([run, runB]));
    render(<RunConsole />);
    fireEvent.click(await screen.findByRole("button", { name: "Open run run-1" }));
    fireEvent.click(screen.getByRole("button", { name: "Open run run-2" }));
    await screen.findByTestId("run-state");
    expect(screen.getByTestId("run-state")).toHaveTextContent("running");
    await act(async () => { resolveFirst(ok({ ...snapshot, run: { ...run, state: "paused" } })); });
    expect(screen.getByTestId("run-state")).toHaveTextContent("running");
    expect(screen.getByTestId("run-state")).not.toHaveTextContent("paused");
  });

  it("requires a reason to pause, then records it on the durable run", async () => {
    mocks.listWorkflowRuns.mockResolvedValue(ok([run]));
    mocks.pauseWorkflowRun.mockResolvedValue(ok(snapshot));
    render(<RunConsole />);
    fireEvent.click(await screen.findByRole("button", { name: "Open run run-1" }));
    await screen.findByTestId("run-state");
    fireEvent.click(screen.getByRole("button", { name: "Pause run" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/pause reason/i);
    expect(mocks.pauseWorkflowRun).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("Pause reason"), { target: { value: "review pause" } });
    fireEvent.click(screen.getByRole("button", { name: "Pause run" }));
    await waitFor(() => expect(mocks.pauseWorkflowRun).toHaveBeenCalledExactlyOnceWith({
      run_id: "run-1", reason: "review pause",
    }));
  });

  it("blocks resume while an approval gate is open and says why", async () => {
    const gated: RunSnapshotView = { ...snapshot, run: { ...run, state: "awaiting_approval" } };
    mocks.listWorkflowRuns.mockResolvedValue(ok([run]));
    mocks.getWorkflowRun.mockResolvedValue(ok(gated));
    render(<RunConsole />);
    fireEvent.click(await screen.findByRole("button", { name: "Open run run-1" }));
    await screen.findByTestId("run-state");
    expect(screen.queryByRole("button", { name: "Resume run" })).not.toBeInTheDocument();
    expect(screen.getByText(/Resume is blocked while an approval gate is open/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Pause run" })).toBeInTheDocument();
  });

  it("records an approval with the decider and optional resume", async () => {
    mocks.listWorkflowRuns.mockResolvedValue(ok([run]));
    mocks.decideWorkflowApproval.mockResolvedValue(ok(snapshot));
    render(<RunConsole />);
    fireEvent.click(await screen.findByRole("button", { name: "Open run run-1" }));
    fireEvent.click(await screen.findByRole("button", { name: "Approve explore" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/who decided/i);
    expect(mocks.decideWorkflowApproval).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("Decided by (explore)"), { target: { value: "the operator" } });
    fireEvent.click(screen.getByLabelText("Resume the run after approving (explore)"));
    fireEvent.click(screen.getByRole("button", { name: "Approve explore" }));
    await waitFor(() => expect(mocks.decideWorkflowApproval).toHaveBeenCalledExactlyOnceWith({
      run_id: "run-1", approval_id: "appr-1", approved: true, by: "the operator", reason: null, resume_after: true,
    }));
  });

  it("requires a reason to reject, then records the rejection", async () => {
    mocks.listWorkflowRuns.mockResolvedValue(ok([run]));
    mocks.decideWorkflowApproval.mockResolvedValue(ok(snapshot));
    render(<RunConsole />);
    fireEvent.click(await screen.findByRole("button", { name: "Open run run-1" }));
    fireEvent.change(await screen.findByLabelText("Decided by (explore)"), { target: { value: "the operator" } });
    fireEvent.click(screen.getByRole("button", { name: "Reject explore" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/rejection must state a reason/i);
    expect(mocks.decideWorkflowApproval).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("Rejection reason (explore)"), { target: { value: "wrong repository" } });
    fireEvent.click(screen.getByRole("button", { name: "Reject explore" }));
    await waitFor(() => expect(mocks.decideWorkflowApproval).toHaveBeenCalledExactlyOnceWith({
      run_id: "run-1", approval_id: "appr-1", approved: false, by: "the operator", reason: "wrong repository", resume_after: false,
    }));
  });
});

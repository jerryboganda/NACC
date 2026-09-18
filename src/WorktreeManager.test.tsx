import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorktreeLeaseView } from "./bindings";

const mocks = vi.hoisted(() => ({
  listWorktreeLeases: vi.fn(),
}));

vi.mock("./bindings", () => ({ commands: mocks }));
import WorktreeManager from "./WorktreeManager";

const ok = <T,>(data: T) => ({ status: "ok" as const, data });
const err = (error: string) => ({ status: "error" as const, error });

const sampleLeases: WorktreeLeaseView[] = [
  {
    id: "11111111-1111-1111-1111-111111111111",
    project_id: "22222222-2222-2222-2222-222222222222",
    workflow_run_id: "33333333-3333-3333-3333-333333333333",
    node_run_id: null,
    path: "D:\\nacc-worktrees\\run-1",
    branch: "nacc/worktree-feature-1",
    base_commit: "abcdef1234567890abcdef1234567890abcdef12",
    head_commit: "fedcba0987654321fedcba0987654321fedcba09",
    state: "active",
    owner_process_id: 1234,
    quarantine_reason: null,
    created_at_millis: "1735000000000",
    updated_at_millis: "1735000005000",
  },
  {
    id: "44444444-4444-4444-4444-444444444444",
    project_id: "22222222-2222-2222-2222-222222222222",
    workflow_run_id: null,
    node_run_id: null,
    path: "D:\\nacc-worktrees\\quarantined-1",
    branch: "nacc/dirty-branch",
    base_commit: "abcdef1234567890abcdef1234567890abcdef12",
    head_commit: null,
    state: "quarantined",
    owner_process_id: null,
    quarantine_reason: "uncommitted changes detected at run cancellation",
    created_at_millis: "1734990000000",
    updated_at_millis: "1734990010000",
  },
];

describe("WorktreeManager", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders empty state when no leases exist", async () => {
    mocks.listWorktreeLeases.mockResolvedValue(ok([]));
    render(<WorktreeManager />);

    expect(
      await screen.findByTestId("worktree-empty")
    ).toHaveTextContent("No worktree leases match the selected criteria");
  });

  it("renders error state when listWorktreeLeases fails", async () => {
    mocks.listWorktreeLeases.mockResolvedValue(err("sqlite disk I/O error"));
    render(<WorktreeManager />);

    expect(
      await screen.findByTestId("worktree-error")
    ).toHaveTextContent("Failed to retrieve worktree leases: sqlite disk I/O error");
  });

  it("renders worktree leases with states and quarantine details", async () => {
    mocks.listWorktreeLeases.mockResolvedValue(ok(sampleLeases));
    render(<WorktreeManager />);

    await waitFor(() => {
      expect(screen.getByTestId("worktree-row-11111111-1111-1111-1111-111111111111")).toBeInTheDocument();
    });

    expect(screen.getByTestId("lease-state-11111111-1111-1111-1111-111111111111")).toHaveTextContent("active");
    expect(screen.getByTestId("lease-state-44444444-4444-4444-4444-444444444444")).toHaveTextContent("quarantined");
    expect(screen.getByText("nacc/worktree-feature-1")).toBeInTheDocument();
    expect(screen.getByText("uncommitted changes detected at run cancellation")).toBeInTheDocument();
  });

  it("applies filters and requests bounded parameters", async () => {
    mocks.listWorktreeLeases.mockResolvedValue(ok(sampleLeases));
    render(<WorktreeManager />);

    await waitFor(() => {
      expect(mocks.listWorktreeLeases).toHaveBeenCalledWith({
        project_id: null,
        active_only: false,
        limit: 50,
      });
    });

    fireEvent.change(screen.getByTestId("worktree-project-id-input"), {
      target: { value: "22222222-2222-2222-2222-222222222222" },
    });
    fireEvent.click(screen.getByTestId("worktree-active-only-toggle"));
    fireEvent.change(screen.getByTestId("worktree-limit-input"), {
      target: { value: "25" },
    });

    fireEvent.click(screen.getByTestId("worktree-apply-filter-button"));

    await waitFor(() => {
      expect(mocks.listWorktreeLeases).toHaveBeenLastCalledWith({
        project_id: "22222222-2222-2222-2222-222222222222",
        active_only: true,
        limit: 25,
      });
    });
  });
});

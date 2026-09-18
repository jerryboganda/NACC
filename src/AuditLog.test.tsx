import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AuditRecordView } from "./bindings";

const mocks = vi.hoisted(() => ({
  listAuditRecords: vi.fn(),
}));

vi.mock("./bindings", () => ({ commands: mocks }));
import AuditLog from "./AuditLog";

const ok = <T,>(data: T) => ({ status: "ok" as const, data });
const err = (error: string) => ({ status: "error" as const, error });

const sampleRecords: AuditRecordView[] = [
  {
    id: "aaaa1111-aaaa-1111-aaaa-111111111111",
    actor: "orchestrator",
    action: "git_checkout_worktree",
    project_id: "22222222-2222-2222-2222-222222222222",
    workflow_run_id: "33333333-3333-3333-3333-333333333333",
    node_run_id: "44444444-4444-4444-4444-444444444444",
    attempt_id: "55555555-5555-5555-5555-555555555555",
    requested_provider: "codex",
    actual_provider: "codex",
    requested_model: "gpt-5.6-luna",
    actual_model: "gpt-5.6-luna",
    effective_reasoning_level: "high",
    effective_permission_profile: "autonomous_worktree",
    command_executable: "git",
    redacted_arguments: ["worktree", "add", "-b", "feature", "D:\\worktrees\\w1", "[REDACTED:token]"],
    working_directory: "D:\\Projects\\NACC",
    created_at_millis: "1735000000000",
  },
];

describe("AuditLog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders empty state when no audit records match", async () => {
    mocks.listAuditRecords.mockResolvedValue(ok([]));
    render(<AuditLog />);

    expect(
      await screen.findByTestId("audit-empty")
    ).toHaveTextContent("No audit records found for the current query");
  });

  it("renders error state when listAuditRecords fails", async () => {
    mocks.listAuditRecords.mockResolvedValue(err("audit table locked"));
    render(<AuditLog />);

    expect(
      await screen.findByTestId("audit-error")
    ).toHaveTextContent("Failed to retrieve audit records: audit table locked");
  });

  it("renders audit records table with redacted arguments and policy facts", async () => {
    mocks.listAuditRecords.mockResolvedValue(ok(sampleRecords));
    render(<AuditLog />);

    await waitFor(() => {
      expect(screen.getByTestId("audit-row-aaaa1111-aaaa-1111-aaaa-111111111111")).toBeInTheDocument();
    });

    expect(screen.getByText("git_checkout_worktree")).toBeInTheDocument();
    expect(screen.getByText("by orchestrator")).toBeInTheDocument();
    expect(screen.getByText("autonomous_worktree")).toBeInTheDocument();
    expect(screen.getByText("reasoning: high")).toBeInTheDocument();
    expect(screen.getByText("git")).toBeInTheDocument();
    expect(screen.getByText(/\[REDACTED:token\]/)).toBeInTheDocument();
    expect(screen.getByText("cwd: D:\\Projects\\NACC")).toBeInTheDocument();
  });

  it("applies workflow run filter and limit to queries", async () => {
    mocks.listAuditRecords.mockResolvedValue(ok(sampleRecords));
    render(<AuditLog />);

    await waitFor(() => {
      expect(mocks.listAuditRecords).toHaveBeenCalledWith({
        workflow_run_id: null,
        limit: 50,
      });
    });

    fireEvent.change(screen.getByTestId("audit-run-id-input"), {
      target: { value: "33333333-3333-3333-3333-333333333333" },
    });
    fireEvent.change(screen.getByTestId("audit-limit-select"), {
      target: { value: "100" },
    });

    fireEvent.click(screen.getByTestId("audit-apply-filter-button"));

    await waitFor(() => {
      expect(mocks.listAuditRecords).toHaveBeenLastCalledWith({
        workflow_run_id: "33333333-3333-3333-3333-333333333333",
        limit: 100,
      });
    });
  });
});

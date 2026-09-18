import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { QualityGateResultView, ReviewFindingView } from "./bindings";

const mocks = vi.hoisted(() => ({
  listQualityGateResults: vi.fn(),
  listReviewFindings: vi.fn(),
}));

vi.mock("./bindings", () => ({ commands: mocks }));
import QualityReviewCenter from "./QualityReviewCenter";

const ok = <T,>(data: T) => ({ status: "ok" as const, data });
const err = (error: string) => ({ status: "error" as const, error });

const qualityRows: QualityGateResultView[] = [
  {
    workflow_run_id: "11111111-1111-1111-1111-111111111111",
    node_run_id: "22222222-2222-2222-2222-222222222222",
    attempt_id: "33333333-3333-3333-3333-333333333333",
    gate: "frontend-tests",
    command: "npm test -- --run",
    passed: true,
    exit_code: 0,
    timed_out: false,
    duration_ms: "1420",
    log_tail: "42 tests passed",
    created_at_millis: "1735000000000",
  },
];

const reviewRows: ReviewFindingView[] = [
  {
    workflow_run_id: "11111111-1111-1111-1111-111111111111",
    node_run_id: "44444444-4444-4444-4444-444444444444",
    attempt_id: null,
    node_key: "independent-review",
    file: "src/parser.ts",
    line: 87,
    severity: "major",
    summary: "Malformed input can bypass the validation branch",
    evidence: "A concrete malformed fixture reaches the success path at src/parser.ts:87.",
    created_at_millis: "1735000001000",
  },
];

describe("QualityReviewCenter", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listQualityGateResults.mockResolvedValue(ok([]));
    mocks.listReviewFindings.mockResolvedValue(ok([]));
  });

  it("renders explicit empty states for both evidence groups", async () => {
    render(<QualityReviewCenter />);

    expect(await screen.findByTestId("quality-evidence-empty")).toHaveTextContent(
      "No quality-gate results found",
    );
    expect(screen.getByTestId("review-findings-empty")).toHaveTextContent(
      "No review findings found",
    );
  });

  it("renders deterministic gate facts and structured review findings", async () => {
    mocks.listQualityGateResults.mockResolvedValue(ok(qualityRows));
    mocks.listReviewFindings.mockResolvedValue(ok(reviewRows));
    render(<QualityReviewCenter />);

    expect(await screen.findByText("frontend-tests")).toBeInTheDocument();
    expect(screen.getByText("Passed")).toBeInTheDocument();
    expect(screen.getByText("npm test -- --run")).toBeInTheDocument();
    expect(screen.getByText("1.42 s")).toBeInTheDocument();

    expect(screen.getByText("major")).toBeInTheDocument();
    expect(screen.getByText("src/parser.ts:87")).toBeInTheDocument();
    expect(
      screen.getByText("Malformed input can bypass the validation branch"),
    ).toBeInTheDocument();
  });

  it("keeps one evidence stream usable when the other returns an error", async () => {
    mocks.listQualityGateResults.mockResolvedValue(err("quality table locked"));
    mocks.listReviewFindings.mockResolvedValue(ok(reviewRows));
    render(<QualityReviewCenter />);

    expect(await screen.findByTestId("quality-evidence-error")).toHaveTextContent(
      "quality table locked",
    );
    expect(screen.getByText("Malformed input can bypass the validation branch")).toBeInTheDocument();
    expect(screen.queryByTestId("review-findings-error")).not.toBeInTheDocument();
  });

  it("applies workflow, node, and limit filters to both bounded queries", async () => {
    render(<QualityReviewCenter />);

    await waitFor(() => {
      expect(mocks.listQualityGateResults).toHaveBeenCalledWith({
        workflow_run_id: null,
        node_run_id: null,
        limit: 50,
      });
      expect(mocks.listReviewFindings).toHaveBeenCalledWith({
        workflow_run_id: null,
        node_run_id: null,
        limit: 50,
      });
    });

    fireEvent.change(screen.getByTestId("evidence-run-id-input"), {
      target: { value: "11111111-1111-1111-1111-111111111111" },
    });
    fireEvent.change(screen.getByTestId("evidence-node-id-input"), {
      target: { value: "22222222-2222-2222-2222-222222222222" },
    });
    fireEvent.change(screen.getByTestId("evidence-limit-select"), {
      target: { value: "100" },
    });
    fireEvent.click(screen.getByTestId("evidence-apply-filter-button"));

    const expected = {
      workflow_run_id: "11111111-1111-1111-1111-111111111111",
      node_run_id: "22222222-2222-2222-2222-222222222222",
      limit: 100,
    };
    await waitFor(() => {
      expect(mocks.listQualityGateResults).toHaveBeenLastCalledWith(expected);
      expect(mocks.listReviewFindings).toHaveBeenLastCalledWith(expected);
    });
  });
});

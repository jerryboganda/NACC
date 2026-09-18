import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  WorkflowTemplateDefinitionView,
  WorkflowTemplateView,
} from "./bindings";

const mocks = vi.hoisted(() => ({
  listWorkflowTemplates: vi.fn(),
  getWorkflowTemplate: vi.fn(),
}));

vi.mock("./bindings", () => ({ commands: mocks }));
import WorkflowInspector from "./WorkflowInspector";

const ok = <T,>(data: T) => ({ status: "ok" as const, data });
const err = (error: string) => ({ status: "error" as const, error });

const sampleTemplates: WorkflowTemplateView[] = [
  {
    name: "autonomous_feature",
    description: "Multi-role autonomous engineering pipeline",
    node_keys: ["plan", "implement", "review"],
    version: 2,
    is_built_in: true,
  },
  {
    name: "custom_audit",
    description: "Custom security analysis pipeline",
    node_keys: ["scan"],
    version: 1,
    is_built_in: false,
  },
];

const sampleDefinition: WorkflowTemplateDefinitionView = {
  name: "autonomous_feature",
  description: "Multi-role autonomous engineering pipeline",
  version: 2,
  is_built_in: true,
  nodes: [
    {
      key: "plan",
      title: "Architect & Plan",
      role: "architect_planner",
      permission_profile_hint: "plan_only",
      depends_on: [],
      timeout_secs: 600,
      instruction: "Formulate concrete implementation plan",
      retryable: true,
      requires_approval: false,
      fallbacks: [],
    },
    {
      key: "implement",
      title: "Autonomous Coding",
      role: "backend_implementer",
      permission_profile_hint: "autonomous_worktree",
      depends_on: ["plan"],
      timeout_secs: 1800,
      instruction: "Implement strictly per architecture specification",
      retryable: true,
      requires_approval: true,
      fallbacks: [],
    },
  ],
};

describe("WorkflowInspector", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders empty state when no templates exist", async () => {
    mocks.listWorkflowTemplates.mockResolvedValue(ok([]));
    render(<WorkflowInspector />);

    expect(
      await screen.findByTestId("workflow-inspector-empty")
    ).toHaveTextContent("No workflow templates found");
  });

  it("renders error state when listWorkflowTemplates fails", async () => {
    mocks.listWorkflowTemplates.mockResolvedValue(err("storage connection lost"));
    render(<WorkflowInspector />);

    expect(
      await screen.findByTestId("workflow-inspector-error")
    ).toHaveTextContent("Failed to list workflow templates: storage connection lost");
  });

  it("renders detail errors and clears the detail loading state", async () => {
    mocks.listWorkflowTemplates.mockResolvedValue(ok(sampleTemplates));
    mocks.getWorkflowTemplate.mockResolvedValue(err("template definition unavailable"));

    render(<WorkflowInspector />);

    expect(
      await screen.findByTestId("workflow-inspector-error")
    ).toHaveTextContent(
      "Failed to inspect template 'autonomous_feature': template definition unavailable"
    );
    expect(screen.queryByTestId("workflow-detail-loading")).not.toBeInTheDocument();
  });

  it("inspects selected template and renders DAG nodes with permissions", async () => {
    mocks.listWorkflowTemplates.mockResolvedValue(ok(sampleTemplates));
    mocks.getWorkflowTemplate.mockResolvedValue(ok(sampleDefinition));

    render(<WorkflowInspector />);

    await waitFor(() => {
      expect(screen.getByTestId("template-name")).toHaveTextContent("autonomous_feature");
    });

    expect(screen.getByTestId("template-builtin-badge")).toHaveTextContent("Built-in (Protected)");
    expect(screen.getByTestId("node-row-plan")).toBeInTheDocument();
    expect(screen.getByTestId("node-row-implement")).toBeInTheDocument();

    expect(screen.getByText("architect_planner")).toBeInTheDocument();
    expect(screen.getByText("autonomous_worktree")).toBeInTheDocument();
    expect(within(screen.getByTestId("node-row-implement")).getByText("plan")).toBeInTheDocument();
    expect(screen.getByText("Required")).toBeInTheDocument();
  });

  it("allows switching templates via select dropdown", async () => {
    mocks.listWorkflowTemplates.mockResolvedValue(ok(sampleTemplates));
    mocks.getWorkflowTemplate.mockResolvedValue(ok(sampleDefinition));

    render(<WorkflowInspector />);

    await waitFor(() => {
      expect(screen.getByTestId("template-name")).toHaveTextContent("autonomous_feature");
    });

    mocks.getWorkflowTemplate.mockResolvedValue(
      ok({
        name: "custom_audit",
        description: "Custom security analysis pipeline",
        version: 1,
        is_built_in: false,
        nodes: [],
      })
    );

    fireEvent.change(screen.getByTestId("workflow-template-select"), {
      target: { value: "custom_audit" },
    });

    await waitFor(() => {
      expect(screen.getByTestId("template-name")).toHaveTextContent("custom_audit");
    });
    expect(screen.getByTestId("template-builtin-badge")).toHaveTextContent("Custom Template");
  });
});

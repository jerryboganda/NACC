import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// "./bindings" is a generated, gitignored file (see App.tsx's own doc
// comment) that does not exist in this checkout until a Rust build has
// run. Mocking it here means this test suite verifies App's own
// behavior -- loading state, success rendering, error rendering --
// independently of whether a real Rust build is available, which on the
// machine this was authored on, it currently is not (see the Phase 0
// foundation audit). vi.mock is hoisted above the imports below by
// Vitest's transform, so this takes effect before App.tsx's own import
// of "./bindings" resolves.
const getAppDiagnostics = vi.fn();
vi.mock("./bindings", () => ({
  commands: {
    getAppDiagnostics: () => getAppDiagnostics(),
  },
}));

// Imported after the mock is declared so the mocked module is what App
// actually receives.
const { default: App } = await import("./App");

describe("App", () => {
  beforeEach(() => {
    getAppDiagnostics.mockReset();
  });

  it("shows a loading state before the command resolves", () => {
    getAppDiagnostics.mockReturnValue(new Promise(() => {})); // never resolves
    render(<App />);
    expect(screen.getByTestId("diagnostics-loading")).toBeInTheDocument();
  });

  it("renders the diagnostics once the command resolves", async () => {
    getAppDiagnostics.mockResolvedValue({
      app_version: "0.0.1",
      workspace_crate_count: 21,
      minimum_rust_version: "1.90",
      dev_mode: true,
      sample_workflow_run_id: "5b6f2b2e-8f2b-4b0c-9e0a-1a2b3c4d5e6f",
    });

    render(<App />);

    await waitFor(() => expect(screen.getByTestId("diagnostics")).toBeInTheDocument());

    expect(screen.getByTestId("app-version")).toHaveTextContent("0.0.1");
    expect(screen.getByTestId("workspace-crate-count")).toHaveTextContent("21");
    expect(screen.getByTestId("minimum-rust-version")).toHaveTextContent("1.90");
    expect(screen.getByTestId("dev-mode")).toHaveTextContent("yes");
    expect(screen.getByTestId("sample-workflow-run-id")).toHaveTextContent(
      "5b6f2b2e-8f2b-4b0c-9e0a-1a2b3c4d5e6f",
    );
  });

  it("renders an accessible error message when the command rejects", async () => {
    getAppDiagnostics.mockRejectedValue(new Error("backend unavailable"));

    render(<App />);

    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
    expect(screen.getByTestId("diagnostics-error")).toHaveTextContent("backend unavailable");
  });
});

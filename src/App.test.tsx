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

  it("renders the diagnostics once the command resolves with status ok", async () => {
    // get_app_diagnostics returns Result<AppDiagnostics, String> on the
    // Rust side; tauri-specta 2.0.0-rc.25 wraps that as a *resolved*
    // {status:"ok"|"error"} discriminated union rather than a rejected
    // promise (verified against tauri-specta's own codegen source -- see
    // App.tsx's doc comment) -- this is the realistic success shape, not
    // the bare object Phase 1 used before the command did real storage
    // I/O and could fail.
    getAppDiagnostics.mockResolvedValue({
      status: "ok",
      data: {
        app_version: "0.0.1",
        workspace_crate_count: 21,
        minimum_rust_version: "1.93",
        dev_mode: true,
        sample_workflow_run_id: "5b6f2b2e-8f2b-4b0c-9e0a-1a2b3c4d5e6f",
        storage_schema_version: "2",
        diagnostics_requests_recorded: 1,
      },
    });

    render(<App />);

    await waitFor(() => expect(screen.getByTestId("diagnostics")).toBeInTheDocument());

    expect(screen.getByTestId("app-version")).toHaveTextContent("0.0.1");
    expect(screen.getByTestId("workspace-crate-count")).toHaveTextContent("21");
    expect(screen.getByTestId("minimum-rust-version")).toHaveTextContent("1.93");
    expect(screen.getByTestId("dev-mode")).toHaveTextContent("yes");
    expect(screen.getByTestId("sample-workflow-run-id")).toHaveTextContent(
      "5b6f2b2e-8f2b-4b0c-9e0a-1a2b3c4d5e6f",
    );
    expect(screen.getByTestId("storage-schema-version")).toHaveTextContent("2");
    expect(screen.getByTestId("diagnostics-requests-recorded")).toHaveTextContent("1");
  });

  it("renders an accessible error message when the command resolves with status error", async () => {
    // The realistic error path for our own command: get_app_diagnostics's
    // internal .map_err(|e| e.to_string()) turns a real StorageError into
    // this resolved status:"error" shape, not a rejection.
    getAppDiagnostics.mockResolvedValue({
      status: "error",
      error: "sqlite error: database is locked",
    });

    render(<App />);

    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
    expect(screen.getByTestId("diagnostics-error")).toHaveTextContent(
      "sqlite error: database is locked",
    );
  });

  it("renders an accessible error message when the command promise itself rejects", async () => {
    // A genuine transport/JS-level failure (e.g. the webview's IPC bridge
    // itself throwing) is still a real possibility distinct from a
    // Rust-side Err, and still must render the same accessible error UI.
    getAppDiagnostics.mockRejectedValue(new Error("backend unavailable"));

    render(<App />);

    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
    expect(screen.getByTestId("diagnostics-error")).toHaveTextContent("backend unavailable");
  });
});

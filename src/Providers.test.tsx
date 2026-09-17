import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { ProviderInstallationView } from "./bindings";
const mocks = vi.hoisted(() => ({ listProviderInstallations: vi.fn(), detectProvider: vi.fn() }));
vi.mock("./bindings", () => ({ commands: mocks }));
import Providers from "./Providers";
const observation: ProviderInstallationView = {
  provider: "claude", runtime: "native_windows", installed: true,
  executable_path: "claude", version: "CLI reported version", detected_at_millis: "9007199254740993",
};
beforeEach(() => {
  vi.resetAllMocks();
  mocks.listProviderInstallations.mockResolvedValue({ status: "ok", data: [] });
});
it("loads saved facts without launching a probe and identifies unverified readiness", async () => {
  mocks.listProviderInstallations.mockResolvedValue({ status: "ok", data: [observation] });
  render(<Providers />);
  expect(screen.getByText("Loading saved detections…")).toBeInTheDocument();
  await screen.findByText(observation.version!);
  expect(screen.getByText(observation.detected_at_millis)).toBeInTheDocument();
  expect(screen.getByText(/not authentication, model availability, or readiness/i)).toBeInTheDocument();
  expect(mocks.detectProvider).not.toHaveBeenCalled();
});
it("detects only on request, disables duplicate clicks, and renders returned observations", async () => {
  let resolve!: (value: unknown) => void;
  mocks.detectProvider.mockReturnValue(new Promise(done => { resolve = done; }));
  render(<Providers />);
  await screen.findByText("No saved provider detections.");
  fireEvent.click(screen.getByRole("button", { name: "Detect claude" }));
  expect(screen.getByRole("button", { name: "Detect codex" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Detecting claude…" }));
  expect(mocks.detectProvider).toHaveBeenCalledExactlyOnceWith({ provider_id: "claude" });
  await act(async () => { resolve({ status: "ok", data: observation }); });
  expect(await screen.findByText("Version probe succeeded")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Detect codex" })).toBeEnabled();
});
it("keeps the previous observation when detection fails", async () => {
  mocks.listProviderInstallations.mockResolvedValue({ status: "ok", data: [observation] });
  mocks.detectProvider.mockResolvedValue({ status: "error", error: "detection timed out" });
  render(<Providers />);
  await screen.findByText(observation.version!);
  fireEvent.click(screen.getByRole("button", { name: "Detect claude" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("detection timed out");
  expect(screen.getByText(observation.version!)).toBeInTheDocument();
});
it("renders unsuccessful probes honestly and lets a failed load be retried", async () => {
  mocks.listProviderInstallations.mockRejectedValueOnce(new Error("storage unavailable"));
  render(<Providers />);
  expect(await screen.findByRole("alert")).toHaveTextContent("storage unavailable");
  mocks.listProviderInstallations.mockResolvedValue({ status: "ok", data: [{ ...observation, installed: false, version: null, executable_path: null }] });
  fireEvent.click(screen.getByRole("button", { name: "Reload saved detections" }));
  expect(await screen.findByText("Not detected or probe unsuccessful")).toBeInTheDocument();
  expect(screen.getByText("Version unavailable")).toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

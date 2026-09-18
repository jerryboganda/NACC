import { StrictMode } from "react";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RoleProfileView } from "./bindings";

const mocks = vi.hoisted(() => ({
  listRoleProfiles: vi.fn(), createRoleProfile: vi.fn(), updateRoleProfile: vi.fn(),
  setRoleProfileEnabled: vi.fn(), deleteRoleProfile: vi.fn(), latestProviderCapabilities: vi.fn(),
}));
vi.mock("./bindings", () => ({ commands: mocks }));
import RoleMatrix from "./RoleMatrix";

const profile: RoleProfileView = {
  id: "test-role", name: "Explorer", role_kind: { custom: "Investigator" },
  provider_id: "claude", model_id: null, thinking_mode: "auto",
  reasoning_level: "auto", permission_profile: "read_only", enabled: true,
  account_label: null, fallbacks: [],
  created_at_millis: "9007199254740993", updated_at_millis: "9007199254740994",
};
const ok = <T,>(data: T) => ({ status: "ok", data });

beforeEach(() => {
  vi.resetAllMocks();
  mocks.listRoleProfiles.mockResolvedValue(ok([]));
  mocks.latestProviderCapabilities.mockResolvedValue(ok(null));
});

describe("Role Matrix", () => {
  it("ignores a stale StrictMode load response", async () => {
    let resolveFirst!: (value: ReturnType<typeof ok<RoleProfileView[]>>) => void;
    mocks.listRoleProfiles.mockReturnValueOnce(new Promise(resolve => { resolveFirst = resolve; }))
      .mockResolvedValueOnce(ok([profile]));
    render(<StrictMode><RoleMatrix /></StrictMode>);
    await screen.findByRole("button", { name: "Edit Explorer" });
    await act(async () => { resolveFirst(ok([])); });
    expect(screen.getByRole("button", { name: "Edit Explorer" })).toBeInTheDocument();
  });

  it("reports load errors and retries without hiding the failure", async () => {
    mocks.listRoleProfiles.mockRejectedValueOnce(new Error("IPC unavailable"));
    render(<RoleMatrix />);
    expect(await screen.findByRole("alert")).toHaveTextContent("IPC unavailable");
    mocks.listRoleProfiles.mockResolvedValue(ok([profile]));
    fireEvent.click(screen.getByRole("button", { name: "Refresh profiles" }));
    await screen.findByRole("button", { name: "Edit Explorer" });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("keeps the draft on a backend save error and blocks duplicate submission", async () => {
    let rejectSave!: (value: { status: string; error: string }) => void;
    mocks.createRoleProfile.mockReturnValue(new Promise(resolve => { rejectSave = resolve; }));
    render(<RoleMatrix />);
    await screen.findByText("No role profiles saved.");
    fireEvent.change(screen.getByLabelText("Profile name"), { target: { value: "Keep me" } });
    const form = screen.getByRole("button", { name: "Save profile" }).closest("form")!;
    fireEvent.submit(form);
    fireEvent.submit(form);
    expect(mocks.createRoleProfile).toHaveBeenCalledTimes(1);
    await act(async () => { rejectSave({ status: "error", error: "database locked" }); });
    expect(await screen.findByRole("alert")).toHaveTextContent("database locked");
    expect(screen.getByLabelText("Profile name")).toHaveValue("Keep me");
    expect(screen.getByRole("button", { name: "Save profile" })).toBeEnabled();
  });

  it("shows loading, empty state, and disabled unvalidated capability controls", async () => {
    render(<RoleMatrix />);
    expect(screen.getByText("Loading role profiles…")).toBeInTheDocument();
    await screen.findByText("No role profiles saved.");
    expect(screen.getByLabelText("Thinking")).toBeDisabled();
    expect(screen.getByLabelText("Reasoning effort")).toBeDisabled();
    expect(screen.queryByRole("option", { name: /temporary danger/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "copilot" })).not.toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "antigravity" })).not.toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "opencode" })).not.toBeInTheDocument();
  });

  it("creates an unassigned profile through the typed command", async () => {
    mocks.createRoleProfile.mockImplementation(async args => ok({ profile: { ...profile, ...args } }));
    render(<RoleMatrix />);
    await screen.findByText("No role profiles saved.");
    fireEvent.change(screen.getByLabelText("Profile name"), { target: { value: "  New explorer  " } });
    fireEvent.click(screen.getByRole("button", { name: "Save profile" }));
    await screen.findByRole("button", { name: "Edit New explorer" });
    expect(mocks.createRoleProfile).toHaveBeenCalledExactlyOnceWith({
      name: "New explorer", role_kind: "repository_explorer", provider_id: null, model_id: null,
      thinking_mode: "auto", reasoning_level: "auto", permission_profile: "read_only",
      account_label: null, fallbacks: [],
    });
  });

  it("keeps thinking and reasoning disabled without a verified capability snapshot", async () => {
    render(<RoleMatrix />);
    await screen.findByText("No role profiles saved.");
    expect(screen.getByLabelText("Thinking")).toBeDisabled();
    expect(screen.getByLabelText("Reasoning effort")).toBeDisabled();
    // Choosing a provider with no snapshot on file must not enable either
    // control: absence of evidence is "disabled with an explanation", never
    // a guessed default (S10.1, acceptance 11).
    fireEvent.change(screen.getByLabelText("Provider"), { target: { value: "codex" } });
    await waitFor(() => expect(mocks.latestProviderCapabilities).toHaveBeenCalledExactlyOnceWith({ provider_id: "codex" }));
    expect(screen.getByLabelText("Thinking")).toBeDisabled();
    expect(screen.getByLabelText("Reasoning effort")).toBeDisabled();
  });

  it("enables verified reasoning levels once a capability snapshot lists them", async () => {
    mocks.latestProviderCapabilities.mockResolvedValue(ok({
      provider: "codex", installed: true, version: "0.149.1", authenticated: true,
      health: "ready", checked_at_millis: "42",
      models: [{
        id: "gpt-5-codex", display_name: "gpt-5-codex",
        reasoning_levels: ["minimal", "low", "medium", "high", "xhigh"],
        thinking: "unsupported", context_window_tokens: null,
      }],
    }));
    mocks.createRoleProfile.mockImplementation(async args => ok({ profile: { ...profile, ...args } }));
    render(<RoleMatrix />);
    await screen.findByText("No role profiles saved.");
    // The gating is model-aware: choose provider AND model, then the
    // reasoning control carries exactly the verified levels (S10.1/S10.2).
    fireEvent.change(screen.getByLabelText("Provider"), { target: { value: "codex" } });
    await screen.findByRole("option", { name: "gpt-5-codex" });
    fireEvent.change(screen.getByLabelText("Requested model ID"), { target: { value: "gpt-5-codex" } });
    const reasoning = await screen.findByLabelText("Reasoning effort");
    await waitFor(() => expect(reasoning).toBeEnabled());
    fireEvent.change(reasoning, { target: { value: "xhigh" } });
    fireEvent.change(screen.getByLabelText("Profile name"), { target: { value: "Coded" } });
    fireEvent.click(screen.getByRole("button", { name: "Save profile" }));
    await screen.findByRole("button", { name: "Edit Coded" });
    expect(mocks.createRoleProfile).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      reasoning_level: "xhigh", model_id: "gpt-5-codex",
    }));
  });

  it("changes provider while clearing provider-specific model controls and preserving role and enabled state", async () => {
    mocks.listRoleProfiles.mockResolvedValue(ok([profile]));
    mocks.updateRoleProfile.mockImplementation(async (id, update) => ok({ profile: { ...profile, id, ...update } }));
    render(<RoleMatrix />);
    fireEvent.click(await screen.findByRole("button", { name: "Edit Explorer" }));
    expect(screen.getByLabelText("Custom role name")).toHaveValue("Investigator");
    fireEvent.change(screen.getByLabelText("Provider"), { target: { value: "codex" } });
    fireEvent.click(screen.getByRole("button", { name: "Save profile" }));
    await waitFor(() => expect(mocks.updateRoleProfile).toHaveBeenCalledExactlyOnceWith(profile.id, {
      name: profile.name, role_kind: profile.role_kind, provider_id: "codex", model_id: null,
      thinking_mode: "auto", reasoning_level: "auto", permission_profile: "read_only",
      account_label: null, fallbacks: [], enabled: true,
    }));
    await screen.findByText("New role profile");
  });

  it("renders an unsupported legacy provider only as a repairable disabled value", async () => {
    const legacy = { ...profile, provider_id: "copilot" as const, model_id: "legacy-model" };
    mocks.listRoleProfiles.mockResolvedValue(ok([legacy]));
    render(<RoleMatrix />);
    fireEvent.click(await screen.findByRole("button", { name: "Edit Explorer" }));
    expect(screen.getByRole("option", { name: "copilot — unavailable in this build" })).toBeDisabled();
    expect(screen.getByRole("option", { name: "legacy-model — unverified legacy value" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Save profile" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("copilot is not runnable in this build");
    expect(mocks.updateRoleProfile).not.toHaveBeenCalled();
  });

  it("persists disabling and requires confirmation before deletion", async () => {
    mocks.listRoleProfiles.mockResolvedValue(ok([profile]));
    mocks.setRoleProfileEnabled.mockResolvedValue(ok(null));
    mocks.deleteRoleProfile.mockResolvedValue(ok(true));
    render(<RoleMatrix />);
    fireEvent.click(await screen.findByRole("button", { name: "Disable Explorer" }));
    await screen.findByRole("button", { name: "Enable Explorer" });
    expect(mocks.setRoleProfileEnabled).toHaveBeenCalledExactlyOnceWith(profile.id, false);
    fireEvent.click(screen.getByRole("button", { name: "Delete Explorer" }));
    expect(mocks.deleteRoleProfile).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Keep profile" }));
    expect(mocks.deleteRoleProfile).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Delete Explorer" }));
    fireEvent.click(screen.getByRole("button", { name: "Confirm delete" }));
    await screen.findByText("No role profiles saved.");
    expect(mocks.deleteRoleProfile).toHaveBeenCalledExactlyOnceWith(profile.id);
  });
});

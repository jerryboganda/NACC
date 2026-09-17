import { StrictMode } from "react";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RoleProfileView } from "./bindings";

const mocks = vi.hoisted(() => ({
  listRoleProfiles: vi.fn(), createRoleProfile: vi.fn(), updateRoleProfile: vi.fn(),
  setRoleProfileEnabled: vi.fn(), deleteRoleProfile: vi.fn(),
}));
vi.mock("./bindings", () => ({ commands: mocks }));
import RoleMatrix from "./RoleMatrix";

const profile: RoleProfileView = {
  id: "test-role", name: "Explorer", role_kind: { custom: "Investigator" },
  provider_id: "claude", model_id: "user-configured-model", thinking_mode: "on",
  reasoning_level: "high", permission_profile: "read_only", enabled: true,
  created_at_millis: "9007199254740993", updated_at_millis: "9007199254740994",
};
const ok = <T,>(data: T) => ({ status: "ok", data });

beforeEach(() => {
  vi.resetAllMocks();
  mocks.listRoleProfiles.mockResolvedValue(ok([]));
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
    });
  });

  it("changes provider independently and preserves custom role, model, reasoning and enabled state", async () => {
    mocks.listRoleProfiles.mockResolvedValue(ok([profile]));
    mocks.updateRoleProfile.mockImplementation(async (id, update) => ok({ profile: { ...profile, id, ...update } }));
    render(<RoleMatrix />);
    fireEvent.click(await screen.findByRole("button", { name: "Edit Explorer" }));
    expect(screen.getByLabelText("Custom role name")).toHaveValue("Investigator");
    fireEvent.change(screen.getByLabelText("Provider"), { target: { value: "codex" } });
    fireEvent.click(screen.getByRole("button", { name: "Save profile" }));
    await waitFor(() => expect(mocks.updateRoleProfile).toHaveBeenCalledExactlyOnceWith(profile.id, {
      name: profile.name, role_kind: profile.role_kind, provider_id: "codex", model_id: profile.model_id,
      thinking_mode: "on", reasoning_level: "high", permission_profile: "read_only", enabled: true,
    }));
    await screen.findByText("New role profile");
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

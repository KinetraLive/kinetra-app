// Permissions (presets, summaries) and the invite dialog: invite with a preset or custom boxes,
// the limit, edit and remove, and the API's refusals in words.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Collaborator, Collaborators } from "$lib/types";
import { normalize, presetOf, summaryOf } from "./permissions";

const kari: Collaborator = {
  id: "u1",
  status: "joined",
  email: "kari@example.com",
  name: "Kari Nordmann",
  permissions: ["edit_fields", "rows", "watch_preview"],
  invitedAt: "2026-10-27T10:00:00Z",
  joinedAt: "2026-10-27T10:05:00Z",
  expiresAt: null,
};
const jonas: Collaborator = {
  id: "i2",
  status: "pending",
  email: "jonas@example.com",
  name: null,
  permissions: ["watch_preview"],
  invitedAt: "2026-10-27T11:00:00Z",
  joinedAt: null,
  expiresAt: "2026-11-03T11:00:00Z",
};
const listOf = (cs: Collaborator[], max = 5): Collaborators => ({ showId: "show-1", limit: { max, used: cs.length }, collaborators: cs });

const api = vi.hoisted(() => ({
  remoteCollaborators: vi.fn(),
  remoteInvite: vi.fn(),
  remoteCollaboratorSet: vi.fn(),
  remoteCollaboratorRemove: vi.fn(async () => {}),
}));
vi.mock("$lib/api/remote", () => api);
vi.mock("$lib/api", () => ({
  toAppError: (e: unknown) => (e && typeof e === "object" && "code" in e ? e : { code: "internal", message: String(e) }),
}));
vi.mock("$lib/components/license/open", () => ({ openInBrowser: vi.fn() }));

import InviteDialog from "./InviteDialog.svelte";
import { openInBrowser } from "$lib/components/license/open";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("permissions", () => {
  it("normalizes codes: known only, in order, editing live brings editing", () => {
    expect(normalize(["rows", "future", "take_clear", "rows"])).toEqual(["take_clear", "rows"]);
    expect(normalize(["edit_live_fields"])).toEqual(["edit_fields", "edit_live_fields"]);
  });

  it("names presets exactly, else Custom", () => {
    expect(presetOf(["watch_preview"])).toBe("watch");
    expect(presetOf(["rows", "watch_preview", "edit_fields"])).toBe("data");
    expect(presetOf(["rows"])).toBeNull();
    expect(summaryOf({ permissions: ["watch_preview", "unknown"] })).toBe("Watch only");
    expect(summaryOf({ permissions: ["rows"] })).toBe("Custom");
    expect(summaryOf({ permissions: [] })).toBe("No permissions");
    expect(summaryOf({ permissions: null, role: "operator" })).toBe("Full operator");
  });
});

async function openDialog(list: Collaborators, crewUrl: string | null = null) {
  api.remoteCollaborators.mockResolvedValue(list);
  render(InviteDialog, { open: true, onclose: () => {}, crewUrl });
  await waitFor(() => expect(api.remoteCollaborators).toHaveBeenCalled());
}

describe("InviteDialog", () => {
  it("invites with Watch only by default (least privilege), or the boxes picked", async () => {
    await openDialog(listOf([]));
    expect(await screen.findByText("No one yet.")).toBeTruthy();
    expect(screen.getByText("0 of 5 collaborators")).toBeTruthy();
    api.remoteInvite.mockResolvedValue({ ...jonas });
    await fireEvent.input(screen.getByLabelText("Email"), { target: { value: "jonas@example.com" } });
    await fireEvent.click(screen.getByRole("button", { name: "Send invite" }));
    expect(api.remoteInvite).toHaveBeenCalledWith("jonas@example.com", ["watch_preview"]);
    await waitFor(() => expect(api.remoteCollaborators).toHaveBeenCalledTimes(2));

    // Watch only, then a box more: Custom.
    await fireEvent.click(screen.getByLabelText("Watch only"));
    await fireEvent.click(screen.getByLabelText("Run cues"));
    expect(screen.getByText("Custom").getAttribute("aria-hidden")).toBe("false");
    await fireEvent.input(screen.getByLabelText("Email"), { target: { value: "ola@example.com" } });
    await fireEvent.click(screen.getByRole("button", { name: "Send invite" }));
    expect(api.remoteInvite).toHaveBeenLastCalledWith("ola@example.com", ["run_cues", "watch_preview"]);
  });

  it("ticking live edits ticks edits; unticking edits drops live edits", async () => {
    await openDialog(listOf([]));
    await fireEvent.click(screen.getByLabelText("Watch only"));
    const live = screen.getByLabelText("Edit graphics on air") as HTMLInputElement;
    const off = screen.getByLabelText("Edit graphics that aren't on air") as HTMLInputElement;
    await fireEvent.click(live);
    expect(off.checked && live.checked).toBe(true);
    await fireEvent.click(off);
    expect(off.checked || live.checked).toBe(false);
  });

  it("says the API's refusals in words", async () => {
    await openDialog(listOf([kari], 1));
    // At the limit: Send invite is off.
    expect((screen.getByRole("button", { name: "Send invite" }) as HTMLButtonElement).disabled).toBe(true);
    cleanup();
    await openDialog(listOf([kari]));
    api.remoteInvite.mockRejectedValue({ code: "cloud", apiCode: "collaborator_limit", message: "x", details: { max: 3 } });
    await fireEvent.input(screen.getByLabelText("Email"), { target: { value: "a@b.no" } });
    await fireEvent.click(screen.getByRole("button", { name: "Send invite" }));
    expect(await screen.findByText("Your plan allows 3 collaborators per show.")).toBeTruthy();
    api.remoteInvite.mockRejectedValue({ code: "cloud", apiCode: "already_collaborator", message: "x" });
    await fireEvent.click(screen.getByRole("button", { name: "Send invite" }));
    expect(await screen.findByText("Already a collaborator. Change their permissions instead.")).toBeTruthy();
  });

  it("lists joined then pending, edits and removes", async () => {
    await openDialog(listOf([kari, jonas]));
    const rows = await screen.findAllByRole("listitem");
    expect(within(rows[0]).getByText("Kari Nordmann")).toBeTruthy();
    expect(within(rows[0]).getByText("Data editor")).toBeTruthy();
    expect(within(rows[1]).getByText("jonas@example.com")).toBeTruthy();
    expect(within(rows[1]).getByText("Invite sent")).toBeTruthy();
    expect(within(rows[1]).getByText(/^Expires /)).toBeTruthy();

    api.remoteCollaboratorSet.mockResolvedValue(kari);
    await fireEvent.click(within(rows[0]).getByRole("button", { name: "Edit" }));
    await fireEvent.click(within(rows[0]).getByLabelText("Full operator"));
    await fireEvent.click(within(rows[0]).getByRole("button", { name: "Save" }));
    expect(api.remoteCollaboratorSet).toHaveBeenCalledWith("u1", expect.arrayContaining(["take_clear", "button_panel"]));

    const pending = (await screen.findAllByRole("listitem"))[1];
    await fireEvent.click(within(pending).getByRole("button", { name: "Cancel invite" }));
    expect(within(pending).getByText("Cancel this invite?")).toBeTruthy();
    await fireEvent.click(within(pending).getByRole("button", { name: "Cancel invite" }));
    await waitFor(() => expect(api.remoteCollaboratorRemove).toHaveBeenCalledWith("i2"));
  });

  it("offers the control site when the show isn't there", async () => {
    api.remoteCollaborators.mockRejectedValue({ code: "cloud", apiCode: "not_found", message: "x" });
    render(InviteDialog, { open: true, onclose: () => {}, crewUrl: "https://control.kinetra.live/s/x/crew" });
    expect(await screen.findByText("This show isn't on the control site.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Send invite" })).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Open the control site" }));
    expect(openInBrowser).toHaveBeenCalledWith("https://control.kinetra.live/s/x/crew");
  });
});

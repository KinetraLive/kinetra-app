// The version history dialog: days, marks, Keep, Label, Delete (not the newest), Show older, and
// the empty state's real next action.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CloudVersionView } from "$lib/types";

const api = vi.hoisted(() => ({
  cloudVersions: vi.fn(),
  cloudVersionKeep: vi.fn(),
  cloudVersionLabel: vi.fn(),
  cloudVersionDelete: vi.fn(async () => {}),
  cloudBackupNow: vi.fn(async () => null),
}));
vi.mock("$lib/api/cloud-history", () => api);
vi.mock("$lib/api/cloud-backup", () => api);
vi.mock("$lib/api", () => ({ on: () => () => {}, toAppError: (e: unknown) => ({ message: String(e) }) }));
vi.mock("$lib/api/cloud", () => ({ cloudView: vi.fn(() => new Promise(() => {})) }));

import HistoryDialog from "./HistoryDialog.svelte";
import { cloud } from "../state.svelte";

const v = (id: string, patch: Partial<CloudVersionView> = {}): CloudVersionView => ({
  id,
  createdAt: "2026-10-20T12:02:00Z",
  machine: "STUDIO-PC",
  label: null,
  kept: false,
  isHead: false,
  damaged: false,
  conflict: false,
  fileCount: 3,
  logicalBytes: 5000,
  newBytes: 0,
  reason: "auto",
  expiresAt: null,
  ...patch,
});

beforeEach(() => {
  HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
    this.setAttribute("open", "");
  };
  HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
    this.removeAttribute("open");
  };
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  cloud.view = null;
});

const rowOf = (text: string) => screen.getByText(text).closest("li") as HTMLElement;

describe("version history", () => {
  it("read-only (lapsed): says until when, no Keep or Label; a shorter history is announced", async () => {
    cloud.view = {
      access: "available",
      state: "lapsed",
      offline: false,
      checkedAt: 1,
      quota: {
        plan: null, usedBytes: 0, pendingBytes: 0, limitBytes: 0, percent: 0, warnPercent: 80, projectsUsed: 1,
        projectsLimit: 0, maxFileBytes: 0, retentionDays: 30, keptPerProject: 0, uploadTodayUsed: 0, uploadTodayLimit: 0,
        uploadTodayResetsAt: null, purgeAt: "2027-01-18T00:00:00Z", retentionDaysNext: 7, retentionFrom: "2026-11-14T00:00:00Z",
      },
      settings: { onAir: "pause", limitKbps: 2000, intervalS: 300, newProjects: true },
      project: null,
      restore: null,
      heldUntil: null,
    };
    api.cloudVersions.mockResolvedValue({ versions: [v("a", { isHead: true }), v("b", { label: "Old" })], next: null });
    render(HistoryDialog, { open: true, onclose: () => {} });
    const row = rowOf(await screen.findByText("Old").then((e) => e.textContent!));
    expect(screen.getByText(/can be downloaded until Jan 1[78], 2027/)).toBeTruthy();
    expect(screen.getByText(/Versions older than 7 days are deleted from Nov 1[34], 2026/)).toBeTruthy();
    expect(within(row).queryByRole("button", { name: "Keep" })).toBeNull();
    expect(within(row).queryByRole("button", { name: "Label" })).toBeNull();
    expect(within(row).getByRole("button", { name: "Delete version" })).toBeTruthy();
  });

  it("lists versions with their marks; the newest can't be deleted", async () => {
    api.cloudVersions.mockResolvedValue({
      versions: [v("a", { isHead: true, label: "After soundcheck" }), v("b", { kept: true, expiresAt: null }), v("c", { expiresAt: "2026-11-14T12:00:00Z" })],
      next: null,
    });
    render(HistoryDialog, { open: true, onclose: () => {} });
    await screen.findByText("After soundcheck");
    const head = rowOf("After soundcheck");
    expect(within(head).getByText("Newest")).toBeTruthy();
    expect(within(head).queryByRole("button", { name: "Delete version" })).toBeNull();
    expect(screen.getByText("Kept")).toBeTruthy();
    expect(screen.getByText("Deleted on Nov 14, 2026")).toBeTruthy();
  });

  it("keeps, labels and deletes", async () => {
    api.cloudVersions.mockResolvedValue({ versions: [v("a", { isHead: true }), v("b", { label: "Old" })], next: null });
    render(HistoryDialog, { open: true, onclose: () => {} });
    const row = rowOf(await screen.findByText("Old").then((e) => e.textContent!));
    api.cloudVersionKeep.mockResolvedValue(v("b", { label: "Old", kept: true }));
    await fireEvent.click(within(row).getByRole("button", { name: "Keep" }));
    await waitFor(() => expect(api.cloudVersionKeep).toHaveBeenCalledWith("b", true));
    // Label.
    await fireEvent.click(within(rowOf("Old")).getByRole("button", { name: "Label" }));
    const input = screen.getByRole("textbox", { name: "Label" }) as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "Before the show" } });
    api.cloudVersionLabel.mockResolvedValue(v("b", { label: "Before the show", kept: true }));
    await fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(api.cloudVersionLabel).toHaveBeenCalledWith("b", "Before the show"));
    expect(await screen.findByText("Before the show")).toBeTruthy();
    // Delete (confirmed inline).
    await fireEvent.click(within(rowOf("Before the show")).getByRole("button", { name: "Delete version" }));
    await fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(api.cloudVersionDelete).toHaveBeenCalledWith("b"));
    await waitFor(() => expect(screen.queryByText("Before the show")).toBeNull());
  });

  it("shows older versions a page at a time", async () => {
    api.cloudVersions
      .mockResolvedValueOnce({ versions: [v("a", { isHead: true, label: "First page" })], next: "a" })
      .mockResolvedValueOnce({ versions: [v("b", { label: "Second page" })], next: null });
    render(HistoryDialog, { open: true, onclose: () => {} });
    await screen.findByText("First page");
    await fireEvent.click(screen.getByRole("button", { name: "Show older" }));
    expect(await screen.findByText("Second page")).toBeTruthy();
    expect(api.cloudVersions).toHaveBeenLastCalledWith("a");
    expect(screen.queryByRole("button", { name: "Show older" })).toBeNull();
  });

  it("offers Back up now when there's no version yet", async () => {
    api.cloudVersions.mockResolvedValue({ versions: [], next: null });
    render(HistoryDialog, { open: true, onclose: () => {} });
    await fireEvent.click(await screen.findByRole("button", { name: "Back up now" }));
    await waitFor(() => expect(api.cloudBackupNow).toHaveBeenCalled());
  });
});

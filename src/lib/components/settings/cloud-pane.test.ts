// Preferences › Cloud backup: status words, storage, this project's switch and its quiet questions,
// and the locked states (signed out, Free). The core decides everything; this only mirrors it.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CloudView } from "$lib/types";

const events = vi.hoisted(() => ({ handlers: new Map<string, (p: unknown) => void>() }));
const api = vi.hoisted(() => ({
  cloudView: vi.fn(),
  cloudRefresh: vi.fn(),
  cloudSetBackup: vi.fn(),
  cloudAnswer: vi.fn(),
  cloudProjectsList: vi.fn(async () => []),
  cloudBackupNow: vi.fn(),
  cloudConflictDismiss: vi.fn(),
}));
const license = vi.hoisted(() => ({ signIn: vi.fn(), start: vi.fn(), view: null }));
vi.mock("$lib/api/cloud", () => api);
vi.mock("$lib/api/cloud-backup", () => api);
vi.mock("$lib/api/cloud-history", () => api);
vi.mock("$lib/api/license", () => ({ cloudLinks: vi.fn(async () => ({ pricing: "https://kinetra.live/pricing" })) }));
vi.mock("$lib/components/license/state.svelte", () => ({ license }));
vi.mock("$lib/api", () => ({
  on: (name: string, h: (p: unknown) => void) => {
    events.handlers.set(name, h);
    return () => events.handlers.delete(name);
  },
  toAppError: (e: unknown) => ({ code: "internal", message: String(e) }),
}));

import CloudPane from "./panes/CloudPane.svelte";
import { cloud } from "$lib/components/cloud/state.svelte";

const base: CloudView = {
  access: "available",
  state: "ok",
  offline: false,
  checkedAt: 1,
  quota: {
    plan: "pro_monthly",
    usedBytes: 7_340_032_000,
    pendingBytes: 0,
    limitBytes: 20e9,
    percent: 36,
    warnPercent: 80,
    projectsUsed: 12,
    projectsLimit: 100,
    maxFileBytes: 5e9,
    retentionDays: 30,
    keptPerProject: 50,
    uploadTodayUsed: 1.2e9,
    uploadTodayLimit: 40e9,
    uploadTodayResetsAt: null,
    purgeAt: null,
    retentionDaysNext: null,
    retentionFrom: null,
  },
  settings: { onAir: "pause", limitKbps: 2000, intervalS: 300, newProjects: true },
  restore: null,
  heldUntil: null,
  project: { backup: true, registered: true, serverId: "p1", noticeSeen: true, question: null, issue: null, run: null, lastBackupAt: null, report: [], skipped: 0, error: null, conflict: null },
};

async function show(v: CloudView) {
  api.cloudView.mockResolvedValue(v);
  render(CloudPane);
  await waitFor(() => expect(api.cloudView).toHaveBeenCalled());
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  cloud.view = null;
});

describe("Cloud backup pane", () => {
  it("shows storage and this project's switch", async () => {
    await show(base);
    expect(await screen.findByText("7.34 GB of 20 GB")).toBeTruthy();
    const sw = screen.getByRole("switch", { name: "Back up this project" });
    api.cloudSetBackup.mockResolvedValue({ ...base, project: { ...base.project!, backup: false } });
    await fireEvent.click(sw);
    await waitFor(() => expect(api.cloudSetBackup).toHaveBeenCalledWith(false));
  });

  it("asks quietly when another computer backs the project up, and sends the answer", async () => {
    await show({
      ...base,
      project: { ...base.project!, registered: false, question: { kind: "otherComputer", machine: "STUDIO-PC", at: null } },
    });
    expect(await screen.findByText("This project is already backed up from STUDIO-PC.")).toBeTruthy();
    api.cloudAnswer.mockResolvedValue(base);
    await fireEvent.click(screen.getByRole("button", { name: "Continue that backup here" }));
    await waitFor(() => expect(api.cloudAnswer).toHaveBeenCalledWith("continueHere"));
    // No dialog: it sits in the pane.
    expect(document.querySelector("dialog")).toBeNull();
  });

  it("follows cloud-changed", async () => {
    await show(base);
    await screen.findByText("7.34 GB of 20 GB");
    events.handlers.get("cloud-changed")?.({ ...base, offline: true });
    expect(await screen.findByText("Cloud backup: offline, will retry.")).toBeTruthy();
  });

  it("offers sign-in when signed out and a check for backups on Free", async () => {
    await show({ ...base, access: "signedOut", state: null, quota: null, project: null });
    await fireEvent.click(await screen.findByRole("button", { name: "Sign in" }));
    expect(license.signIn).toHaveBeenCalled();
    cleanup();
    cloud.view = null;
    await show({ ...base, access: "free", state: null, quota: null, project: null });
    api.cloudRefresh.mockResolvedValue(base);
    await fireEvent.click(await screen.findByRole("button", { name: "Check for backups" }));
    await waitFor(() => expect(api.cloudRefresh).toHaveBeenCalledWith(true));
  });
});

describe("Cloud backup pane: backing up", () => {
  const project = base.project!;

  it("backs up on request and shows progress, the on-air pause and the result", async () => {
    await show({ ...base, project: { ...project, lastBackupAt: null } });
    expect(await screen.findByText("Not backed up yet")).toBeTruthy();
    api.cloudBackupNow.mockResolvedValue({
      ...base,
      project: { ...project, run: { phase: "uploading", doneBytes: 45, totalBytes: 100, paused: false, limited: false } },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Back up now" }));
    await waitFor(() => expect(api.cloudBackupNow).toHaveBeenCalled());
    expect(await screen.findByText("Backing up… 45%")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Back up now" }) as HTMLButtonElement).disabled).toBe(true);
    events.handlers.get("cloud-changed")?.({
      ...base,
      project: { ...project, run: { phase: "uploading", doneBytes: 45, totalBytes: 100, paused: true, limited: false } },
    });
    expect(await screen.findByText("Paused while on air")).toBeTruthy();
    events.handlers.get("cloud-changed")?.({
      ...base,
      project: {
        ...project,
        lastBackupAt: "2026-10-15T12:02:00Z",
        skipped: 2,
        report: [
          { path: "assets/CON.txt", reason: "The name is reserved on Windows." },
          { path: "assets/link", reason: "It's a link; the file it points to isn't backed up." },
        ],
      },
    });
    expect(await screen.findByText(/^Last backup Oct 15/)).toBeTruthy();
    expect(screen.getByText("2 files aren't backed up")).toBeTruthy();
    expect(screen.getByText("assets/CON.txt")).toBeTruthy();
  });

  it("says why a backup stopped and names another computer's backup", async () => {
    await show({
      ...base,
      project: {
        ...project,
        error: "Cloud backup: offline, will retry.",
        conflict: { machine: "STUDIO-PC", at: null, versionId: "v1", otherVersionId: "v0" },
      },
    });
    expect(await screen.findByText("Cloud backup: offline, will retry.")).toBeTruthy();
    expect(screen.getByText("This project was also backed up from STUDIO-PC. Both versions are kept.")).toBeTruthy();
    api.cloudConflictDismiss.mockResolvedValue(base);
    await fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect(api.cloudConflictDismiss).toHaveBeenCalled());
    await waitFor(() => expect(screen.queryByText(/Both versions are kept/)).toBeNull());
  });
});

describe("Cloud backup pane: cloud projects", () => {
  it("asks for the list again after each backup of the open project", async () => {
    await show(base);
    await waitFor(() => expect(api.cloudProjectsList).toHaveBeenCalledTimes(1));
    // A backup finished (from any window): the open project's last backup changes.
    cloud.view = { ...base, project: { ...base.project!, lastBackupAt: "2026-10-21T10:00:00Z" } };
    await waitFor(() => expect(api.cloudProjectsList).toHaveBeenCalledTimes(2));
    // Progress alone doesn't ask again.
    cloud.view = {
      ...base,
      project: {
        ...base.project!,
        lastBackupAt: "2026-10-21T10:00:00Z",
        run: { phase: "uploading", doneBytes: 1, totalBytes: 2, paused: false, limited: false },
      },
    };
    await new Promise((r) => setTimeout(r, 50));
    expect(api.cloudProjectsList).toHaveBeenCalledTimes(2);
  });
});

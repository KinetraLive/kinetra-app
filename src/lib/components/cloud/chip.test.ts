// The top bar's cloud chip: quiet words, the one-time notice, a question's choices, Back up now.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CloudProjectView, CloudView } from "$lib/types";

const api = vi.hoisted(() => ({
  cloudView: vi.fn(),
  cloudAnswer: vi.fn(async () => null),
  cloudBackupNow: vi.fn(async () => null),
  cloudConflictDismiss: vi.fn(async () => null),
  cloudNoticeSeen: vi.fn(async () => undefined),
  cloudSetBackup: vi.fn(async () => null),
}));
vi.mock("$lib/api/cloud", () => api);
vi.mock("$lib/api/cloud-backup", () => api);
vi.mock("$lib/api", () => ({ on: () => () => {}, toAppError: (e: unknown) => ({ message: String(e) }) }));
vi.mock("$lib/components/settings/open", () => ({ preferencesLink: () => () => {} }));
const links = vi.hoisted(() => ({ cloudLinks: vi.fn(async () => ({ pricing: "https://kinetra.live/pricing" })) }));
vi.mock("$lib/api/license", () => links);
const browser = vi.hoisted(() => ({ openInBrowser: vi.fn(async () => {}) }));
vi.mock("$lib/components/license/open", () => browser);
const restoreApi = vi.hoisted(() => ({ cloudRestoreCancel: vi.fn(async () => {}) }));
vi.mock("$lib/api/cloud-restore", () => restoreApi);
const host = vi.hoisted(() => ({ openHistory: vi.fn() }));
vi.mock("./history/host", () => host);

import CloudChip from "./CloudChip.svelte";
import { cloud } from "./state.svelte";

const project: CloudProjectView = {
  backup: true,
  registered: true,
  serverId: "p1",
  noticeSeen: true,
  question: null,
  issue: null,
  run: null,
  lastBackupAt: null,
  report: [],
  skipped: 0,
  error: null,
  conflict: null,
};

function show(p: Partial<CloudProjectView>, v: Partial<CloudView> = {}) {
  cloud.view = {
    access: "available",
    state: "ok",
    offline: false,
    checkedAt: 1,
    quota: null,
    settings: { onAir: "pause", limitKbps: 2000, intervalS: 300, newProjects: true },
    project: { ...project, ...p },
    restore: null,
    heldUntil: null,
    ...v,
  } satisfies CloudView;
  render(CloudChip);
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  cloud.view = null;
});

describe("cloud chip", () => {
  it("shows the one-time notice with OK and Turn off", async () => {
    show({ noticeSeen: false });
    expect(screen.getByRole("button", { name: "Backing up to the cloud" })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "OK", hidden: true }));
    await waitFor(() => expect(api.cloudNoticeSeen).toHaveBeenCalled());
    await fireEvent.click(screen.getByRole("button", { name: "Turn off for this project", hidden: true }));
    await waitFor(() => expect(api.cloudSetBackup).toHaveBeenCalledWith(false));
  });

  it("offers a question's choices", async () => {
    show({ question: { kind: "firstBackup", bytes: 2.4e9 } });
    expect(screen.getByRole("button", { name: "This project's first cloud backup is 2.4 GB." })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Back up", hidden: true }));
    await waitFor(() => expect(api.cloudAnswer).toHaveBeenCalledWith("backUp"));
  });

  it("backs up on request and shows progress quietly", async () => {
    show({});
    await fireEvent.click(screen.getByRole("button", { name: "Back up now", hidden: true }));
    await waitFor(() => expect(api.cloudBackupNow).toHaveBeenCalled());
    cleanup();
    show({ run: { phase: "uploading", doneBytes: 1, totalBytes: 4, paused: false, limited: false } });
    expect(screen.getByRole("button", { name: "Backing up… 25%" })).toBeTruthy();
    expect((screen.getByRole("button", { name: "Back up now", hidden: true }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("opens the other computer's version from the conflict notice", async () => {
    show({ conflict: { machine: "STUDIO-PC", at: null, versionId: "v2", otherVersionId: "v1" } });
    await fireEvent.click(screen.getByRole("button", { name: "Open the other version", hidden: true }));
    expect(host.openHistory).toHaveBeenCalledWith("v1");
  });

  it("read-only (lapsed): no Back up now or Dismiss, the upgrade offer instead", async () => {
    show(
      { conflict: { machine: "STUDIO-PC", at: null, versionId: "v2", otherVersionId: "v1" } },
      { state: "lapsed", quota: null },
    );
    expect(screen.getByRole("button", { name: /no longer includes cloud backup/ })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Back up now", hidden: true })).toBeNull();
    expect(screen.queryByRole("button", { name: "Dismiss", hidden: true })).toBeNull();
    expect(screen.getByRole("button", { name: "Open the other version", hidden: true })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Upgrade to Pro", hidden: true }));
    await waitFor(() => expect(browser.openInBrowser).toHaveBeenCalledWith("https://kinetra.live/pricing"));
  });

  it("shows a restore with the project's backup off, and cancels it", async () => {
    show(
      { backup: false },
      {
        restore: {
          mode: "newFolder",
          versionId: "v1",
          folder: "C:\Shows\gala-old",
          phase: "downloading",
          doneBytes: 1,
          totalBytes: 4,
          paused: false,
          limited: false,
          result: null,
        },
      },
    );
    expect(screen.getByRole("button", { name: "Restoring… 25%" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Back up now", hidden: true })).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Cancel restore", hidden: true }));
    await waitFor(() => expect(restoreApi.cloudRestoreCancel).toHaveBeenCalled());
  });

  it("isn't there when the project doesn't back up", () => {
    show({ backup: false });
    expect(screen.queryByRole("button")).toBeNull();
  });
});

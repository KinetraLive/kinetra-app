// "Back up before quitting?": asked only when the core says so; the three answers; quitting when
// the backup ends, or at once with "Quit now".
import { afterEach, describe, expect, it, vi } from "vitest";

const events = vi.hoisted(() => ({ handlers: new Map<string, (p: unknown) => void>() }));
const api = vi.hoisted(() => ({ cloudExitShouldAsk: vi.fn(), cloudBackupBeforeExit: vi.fn() }));
const dialog = vi.hoisted(() => ({ message: vi.fn() }));
vi.mock("$lib/api/cloud-backup", () => api);
vi.mock("@tauri-apps/plugin-dialog", () => dialog);
vi.mock("$lib/api", () => ({
  on: (name: string, h: (p: unknown) => void) => {
    events.handlers.set(name, h);
    return () => events.handlers.delete(name);
  },
}));

import { toasts } from "$lib/stores/toasts.svelte";
import { backUpBeforeQuit } from "./exit";

const running = { project: { run: { phase: "uploading", doneBytes: 1, totalBytes: 2, paused: false, limited: false } } };

afterEach(() => {
  vi.clearAllMocks();
  events.handlers.clear();
  toasts.list.splice(0);
});

describe("Back up before quitting?", () => {
  it("doesn't ask when there's nothing to back up (or the core can't tell)", async () => {
    const quit = vi.fn(async () => {});
    api.cloudExitShouldAsk.mockResolvedValue(false);
    expect(await backUpBeforeQuit(quit)).toBe(false);
    api.cloudExitShouldAsk.mockRejectedValue(new Error("no backend"));
    expect(await backUpBeforeQuit(quit)).toBe(false);
    expect(dialog.message).not.toHaveBeenCalled();
  });

  it("Quit quits, Cancel stays", async () => {
    const quit = vi.fn(async () => {});
    api.cloudExitShouldAsk.mockResolvedValue(true);
    dialog.message.mockResolvedValue("Quit");
    expect(await backUpBeforeQuit(quit)).toBe(false);
    dialog.message.mockResolvedValue("Cancel");
    expect(await backUpBeforeQuit(quit)).toBe(true);
    expect(api.cloudBackupBeforeExit).not.toHaveBeenCalled();
    expect(quit).not.toHaveBeenCalled();
  });

  it("Back up and quit quits when the backup ends, or at once with Quit now", async () => {
    const quit = vi.fn(async () => {});
    api.cloudExitShouldAsk.mockResolvedValue(true);
    dialog.message.mockResolvedValue("Back up and quit");
    api.cloudBackupBeforeExit.mockResolvedValue(running);
    expect(await backUpBeforeQuit(quit)).toBe(true);
    expect(toasts.list.at(-1)?.message).toBe("Backing up before quitting…");
    events.handlers.get("cloud-changed")?.(running);
    expect(quit).not.toHaveBeenCalled();
    events.handlers.get("cloud-changed")?.({ project: { run: null } });
    expect(quit).toHaveBeenCalledTimes(1);
    expect(toasts.list).toHaveLength(0);
    // Quit now.
    expect(await backUpBeforeQuit(quit)).toBe(true);
    toasts.list.at(-1)?.action?.run();
    expect(quit).toHaveBeenCalledTimes(2);
  });
});

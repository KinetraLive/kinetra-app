import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BackupInfo, ImportPreview, MoveProgress, Prefs, Project, UiState } from "$lib/types";

// ---- Mocks --------------------------------------------------------------------------------------

type Handler = (payload: unknown) => void;

const events = vi.hoisted(() => ({
  handlers: new Map<string, Handler>(),
  unsubs: new Map<string, ReturnType<typeof vi.fn>>(),
}));

const api = vi.hoisted(() => ({
  appInfo: vi.fn(),
  currentProject: vi.fn(),
  updateProjectSettings: vi.fn(),
  getPrefs: vi.fn(),
  setPrefs: vi.fn(),
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  listBackups: vi.fn(),
  createBackup: vi.fn(),
  restoreBackup: vi.fn(),
  deleteBackup: vi.fn(),
  exportSettings: vi.fn(),
  previewSettingsImport: vi.fn(),
  importSettings: vi.fn(),
  moveProject: vi.fn(),
  cancelMove: vi.fn(),
  resetSettings: vi.fn(),
  serverStatus: vi.fn(),
  on: vi.fn(),
  // Server > OSC lists ids for its address list.
  listScreens: vi.fn(async () => []),
  listFlows: vi.fn(async () => []),
  listSources: vi.fn(async () => []),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e
      ? e
      : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
// Domain modules share the same mock functions (the split only changes where they are imported from).
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/backups", () => api);
vi.mock("$lib/api/settings-file", () => api);
vi.mock("$lib/api/move", () => api);
vi.mock("$lib/api/reset", () => api);
vi.mock("$lib/api/flows", () => api);
vi.mock("$lib/api/sources", () => api);
vi.mock("$lib/api/app", () => api);


const flow = vi.hoisted(() => ({ pickFolder: vi.fn(), revealInExplorer: vi.fn() }));
vi.mock("$lib/projectFlow", () => flow);

const dialog = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

const nav = vi.hoisted(() => ({ goto: vi.fn(async (_url: string) => {}), afterNavigate: vi.fn() }));
vi.mock("$app/navigation", () => nav);
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/settings") } }));

// IntersectionObserver stub: tests fire intersections by hand.
const io = vi.hoisted(() => ({
  instances: [] as { cb: IntersectionObserverCallback; disconnected: boolean; targets: Element[] }[],
}));
class FakeIO {
  #rec: (typeof io.instances)[number];
  constructor(cb: IntersectionObserverCallback) {
    this.#rec = { cb, disconnected: false, targets: [] };
    io.instances.push(this.#rec);
  }
  observe(el: Element) {
    this.#rec.targets.push(el);
  }
  unobserve() {}
  disconnect() {
    this.#rec.disconnected = true;
  }
  takeRecords() {
    return [];
  }
}
vi.stubGlobal("IntersectionObserver", FakeIO);

function intersectAll() {
  for (const rec of io.instances) {
    if (rec.disconnected) continue;
    const entries = rec.targets.map((target) => ({ target, isIntersecting: true }));
    rec.cb(entries as unknown as IntersectionObserverEntry[], {} as IntersectionObserver);
  }
}

import AboutSection from "./AboutSection.svelte";
import BackupsSection from "./BackupsSection.svelte";
import GeneralSection from "./GeneralSection.svelte";
import LocationSection from "./LocationSection.svelte";
import ResetSection from "./ResetSection.svelte";
import ServerSection from "./ServerSection.svelte";
import SettingsFileSection from "./SettingsFileSection.svelte";
import { prefs } from "$lib/stores/prefs.svelte";
import { project } from "$lib/stores/project.svelte";
import { server } from "$lib/stores/server.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { uiState } from "$lib/stores/uiState.svelte";
import { settingsBusy } from "./state.svelte";

// ---- Fixtures -----------------------------------------------------------------------------------

const current: Project = {
  path: "C:\\shows\\current",
  name: "Current Show",
  createdAt: "2026-01-02T10:00:00Z",
  settings: { port: 7770, lanAccess: false, defaultResolution: { width: 1920, height: 1080 } },
};

const basePrefs: Prefs = {
  theme: "system",
  backup: { enabled: true, intervalMinutes: 15, retention: 10 },
};

const baseUi: UiState = {
  sidebarPinned: false,
  control: {
    view: "grid",
    gridColumns: 2,
    selectedScreenId: null,
    hiddenScreenIds: [],
    liveEdit: "immediate",
    hotkeys: {},
    automationOff: false,
  },
};

const backups: BackupInfo[] = [
  { id: "a", createdAt: "2026-10-01T10:00:00Z", reason: "timer", files: 12, bytes: 2048 },
  { id: "b", createdAt: "2026-10-02T10:00:00Z", reason: "pre-import", files: 1, bytes: 300 },
];

const err = (code: string, message: string) => ({ code, message });

beforeEach(() => {
  vi.clearAllMocks();
  io.instances.length = 0;
  events.handlers.clear();
  events.unsubs.clear();
  api.on.mockImplementation((name: string, handler: Handler) => {
    events.handlers.set(name, handler);
    const off = vi.fn(() => events.handlers.delete(name));
    events.unsubs.set(name, off);
    return off;
  });
  api.serverStatus.mockResolvedValue({ state: "stopped" });
  api.getPrefs.mockResolvedValue(basePrefs);
  api.setPrefs.mockImplementation(async (p: Prefs) => p);
  api.getUiState.mockResolvedValue(baseUi);
  api.currentProject.mockResolvedValue(current);
  api.listBackups.mockResolvedValue(backups);
  project.current = current;
  prefs.set(basePrefs);
  uiState.set(baseUi);
  settingsBusy.moving = false;
});

afterEach(() => {
  cleanup();
  toasts.clear();
});

const errorToasts = () => toasts.list.filter((t) => t.kind === "error");

/**
 * The port field is sized for 5 digits by NumberInput's `digits` prop, which renders the value in
 * mono at `digits`ch: the input gets the `sized` class and `--digits: 5`.
 */
function expectDigits(input: HTMLInputElement, digits: number) {
  expect(input.classList.contains("sized")).toBe(true);
  expect(input.style.getPropertyValue("--digits")).toBe(String(digits));
}

/** Visible text of an element with whitespace collapsed (values are split into mono spans). */
const text = (el: Element | null) => (el?.textContent ?? "").replace(/\s+/g, " ").trim();

// ---- General ------------------------------------------------------------------------------------

describe("GeneralSection", () => {
  it("has no theme select (the top bar toggle is the control); keeps the default resolution", () => {
    render(GeneralSection);
    expect(screen.queryByLabelText("Theme")).toBeNull();
    expect(screen.queryByRole("option", { name: "System" })).toBeNull();
    const res = screen.getByLabelText("Default screen resolution") as HTMLSelectElement;
    expect(res.value).toBe("1920x1080");
  });

  it("shows width and height fields for a custom size", async () => {
    project.current = {
      ...current,
      settings: { ...current.settings, defaultResolution: { width: 1000, height: 500 } },
    };
    render(GeneralSection);
    expect((screen.getByLabelText("Width") as HTMLInputElement).value).toBe("1000");
    expect((screen.getByLabelText("Height") as HTMLInputElement).value).toBe("500");
    expectDigits(screen.getByLabelText("Width") as HTMLInputElement, 4);
    expectDigits(screen.getByLabelText("Height") as HTMLInputElement, 4);
  });
});

// ---- About --------------------------------------------------------------------------------------

describe("AboutSection", () => {
  it("lists versions and folders in mono once seen, and opens the log folder", async () => {
    api.appInfo.mockResolvedValue({
      version: "0.3.0",
      tauriVersion: "2.1.0",
      webviewVersion: "130.0.2849.80",
      channel: "canary",
      commit: "fa23a08",
      configDir: "C:\\Users\\me\\AppData\\Roaming\\kinetra",
      logDir: "C:\\Users\\me\\AppData\\Local\\kinetra\\logs",
    });
    flow.revealInExplorer.mockResolvedValue(undefined);
    render(AboutSection);
    expect(api.appInfo).not.toHaveBeenCalled();
    intersectAll();
    const version = await screen.findByText("0.3.0");
    expect(version.classList.contains("mono")).toBe(true);
    const terms = [...document.querySelectorAll("dt")].map((dt) => text(dt));
    expect(terms).toEqual(["Version", "Channel", "Commit", "Tauri", "WebView", "Config folder", "Log folder"]);
    // Technical values are mono; the channel is a word.
    for (const dd of document.querySelectorAll("dd")) {
      const label = text(dd.previousElementSibling);
      expect(!!dd.querySelector(".mono")).toBe(label !== "Channel");
    }
    expect(screen.getByText("Canary")).toBeTruthy();
    expect(screen.getByText("fa23a08").classList.contains("mono")).toBe(true);
    await fireEvent.click(screen.getByRole("button", { name: "Open log folder" }));
    expect(flow.revealInExplorer).toHaveBeenCalledWith("C:\\Users\\me\\AppData\\Local\\kinetra\\logs");
  });

  it("opens Third-party licenses lazily and returns focus to the button on close", async () => {
    api.appInfo.mockResolvedValue({ version: "0.3.0", tauriVersion: "2", webviewVersion: "1", configDir: "c", logDir: "l" });
    render(AboutSection);
    expect(screen.queryByRole("dialog", { hidden: true })).toBeNull();
    const button = screen.getByRole("button", { name: "View licenses" });
    button.focus();
    await fireEvent.click(button);
    const dialog = await screen.findByRole("dialog", { name: "Third-party licenses", hidden: true }, { timeout: 5_000 });
    await within(dialog).findByText("@xyflow/svelte", {}, { timeout: 5_000 });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { hidden: true })).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(button));
  });
});

// ---- Backups ------------------------------------------------------------------------------------

describe("BackupsSection", () => {
  it("loads the list only once the section intersects, then shows readable reasons", async () => {
    render(BackupsSection);
    await tick();
    expect(api.listBackups).not.toHaveBeenCalled();
    expect(api.on).not.toHaveBeenCalledWith("backup-created", expect.anything());

    intersectAll();
    await screen.findByText("Automatic");
    expect(api.listBackups).toHaveBeenCalledTimes(1);
    expect(screen.getByText("Before import")).toBeTruthy();
    expect(io.instances.every((r) => r.disconnected)).toBe(true);

    // Newest first. Reason and file count in body text; time and byte size in mono. No row is selected.
    const items = screen.getAllByRole("listitem");
    expect(within(items[0]).getByText("Before import").classList.contains("mono")).toBe(false);
    expect(items[0].querySelector("time")?.classList.contains("mono")).toBe(true);
    const size0 = items[0].querySelector(".size") as HTMLElement;
    expect(size0.textContent).toBe("1 file · 300 B");
    expect(size0.classList.contains("mono")).toBe(false);
    expect(within(items[0]).getByText("300 B").classList.contains("mono")).toBe(true);
    expect(items[1].querySelector(".size")?.textContent).toBe("12 files · 2.0 KB");
    expect(within(items[1]).getByText("2.0 KB").classList.contains("mono")).toBe(true);
    expect(document.querySelector("[aria-selected], [data-selected]")).toBeNull();

    // Subscribed only now; a new snapshot is prepended at once, then the list is re-read so
    // snapshots pruned by retention disappear (here retention dropped "a").
    await waitFor(() => expect(events.handlers.has("backup-created")).toBe(true));
    const c: BackupInfo = {
      id: "c",
      createdAt: "2026-10-03T10:00:00Z",
      reason: "manual",
      files: 3,
      bytes: 10,
    };
    let release!: (list: BackupInfo[]) => void;
    api.listBackups.mockReturnValueOnce(new Promise((r) => (release = r)));
    events.handlers.get("backup-created")!(c);
    await tick();
    expect(within(screen.getAllByRole("listitem")[0]).getByText("Manual")).toBeTruthy();
    expect(screen.getAllByRole("listitem")).toHaveLength(3);

    release([backups[1], c]);
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(2));
    expect(within(screen.getAllByRole("listitem")[0]).getByText("Manual")).toBeTruthy();
    expect(screen.queryByText("Automatic")).toBeNull();
  });

  it("shows an empty line when there are no snapshots", async () => {
    api.listBackups.mockResolvedValue([]);
    render(BackupsSection);
    intersectAll();
    expect(await screen.findByText("No snapshots yet.")).toBeTruthy();
  });

  it("releases the backup-created listener on teardown", async () => {
    const { unmount } = render(BackupsSection);
    intersectAll();
    await screen.findByText("Automatic");
    await waitFor(() => expect(events.unsubs.has("backup-created")).toBe(true));
    unmount();
    expect(events.unsubs.get("backup-created")).toHaveBeenCalled();
  });

  it("sizes the interval and retention fields for three digits", () => {
    render(BackupsSection);
    expectDigits(screen.getByLabelText("Interval") as HTMLInputElement, 3);
    expectDigits(screen.getByLabelText("Keep") as HTMLInputElement, 3);
  });

  it("saves a clamped interval through prefs.update", async () => {
    const update = vi.spyOn(prefs, "update");
    render(BackupsSection);
    const input = screen.getByLabelText("Interval") as HTMLInputElement;
    input.value = "999";
    await fireEvent.change(input);
    expect(update).toHaveBeenCalledWith({
      ...basePrefs,
      backup: { ...basePrefs.backup, intervalMinutes: 240 },
    });
    update.mockRestore();
  });

  it("rolls the interval back and toasts when the save is rejected", async () => {
    api.setPrefs.mockRejectedValue(err("io", "disk full"));
    render(BackupsSection);
    const input = screen.getByLabelText("Interval") as HTMLInputElement;
    input.value = "30";
    await fireEvent.change(input);
    await waitFor(() => expect(errorToasts()).toHaveLength(1));
    expect(errorToasts()[0].message).toContain("disk full");
    await tick();
    expect(input.value).toBe("15");
    expect(prefs.value?.backup.intervalMinutes).toBe(15);
  });
});

// ---- Server -------------------------------------------------------------------------------------

describe("ServerSection", () => {
  it("saves the port through updateProjectSettings", async () => {
    api.updateProjectSettings.mockImplementation(async (settings) => ({ ...current, settings }));
    render(ServerSection);
    const input = screen.getByLabelText("Port") as HTMLInputElement;
    input.value = "8080";
    await fireEvent.change(input);
    await waitFor(() =>
      expect(api.updateProjectSettings).toHaveBeenCalledWith({ ...current.settings, port: 8080 }),
    );
    await waitFor(() => expect(project.current?.settings.port).toBe(8080));
  });

  it("restores the previous port when the save is rejected", async () => {
    api.updateProjectSettings.mockRejectedValue(err("invalid", "Port is in use"));
    render(ServerSection);
    const input = screen.getByLabelText("Port") as HTMLInputElement;
    input.value = "8080";
    await fireEvent.change(input);
    await waitFor(() => expect(errorToasts()).toHaveLength(1));
    await tick();
    expect(input.value).toBe("7770");
    expect(project.current?.settings.port).toBe(7770);
  });

  it("explains LAN access", () => {
    render(ServerSection);
    expect(
      screen.getByText(
        "Other devices can view outputs. Windows may ask once.",
      ),
    ).toBeTruthy();
    expect(screen.queryByText(/arrives in a later version/)).toBeNull();
  });

  it("shows the live status, including a port conflict", async () => {
    server.status = null;
    api.serverStatus.mockResolvedValue({
      state: "error",
      code: "portInUse",
      port: 7770,
      message: "address in use",
    });
    const { container } = render(ServerSection);
    const status = screen.getByRole("status");
    const dot = () => container.querySelector(".tally");
    await waitFor(() =>
      expect(text(status)).toBe("Port 7770 is in use — choose another port."),
    );
    expect(dot()?.classList.contains("warning")).toBe(true);
    expect(within(status).getByText("7770").classList.contains("mono")).toBe(true);

    events.handlers.get("server-status")?.({ state: "starting", port: 7771 });
    await tick();
    expect(text(status)).toBe("Starting on port 7771…");
    expect(dot()?.classList.contains("idle")).toBe(true);

    events.handlers.get("server-status")?.({
      state: "running",
      port: 7771,
      url: "http://127.0.0.1:7771",
      lanUrl: "http://192.168.1.20:7771",
    });
    await tick();
    expect(text(status)).toBe("Running at http://127.0.0.1:7771");
    expect(dot()?.classList.contains("preview")).toBe(true);
    expect(screen.getByText("http://127.0.0.1:7771").classList.contains("mono")).toBe(true);
    expect(screen.getByText("On your network")).toBeTruthy();
    expect(screen.getByText("http://192.168.1.20:7771").classList.contains("mono")).toBe(true);
  });

  it("renders the port field sized for five digits", () => {
    render(ServerSection);
    const input = screen.getByLabelText("Port") as HTMLInputElement;
    expect(input.value).toBe("7770");
    expect(input.max).toBe("65535");
    expectDigits(input, 5);
  });
});

// ---- Import -------------------------------------------------------------------------------------

describe("SettingsFileSection import", () => {
  const preview: ImportPreview = {
    exportedAt: "2026-09-30T12:00:00Z",
    appVersion: "0.1.0",
    sections: [
      {
        section: "prefs",
        changes: [
          { key: "theme", from: "system", to: "dark" },
          { key: "backup.intervalMinutes", from: 15, to: 30 },
        ],
      },
      { section: "project", changes: [] },
      { section: "uiState", changes: [{ key: "control.hotkeys", from: {}, to: { l1: "F1" } }] },
    ],
  };

  it("previews sections, disables unchanged ones and applies only the checked sections", async () => {
    dialog.open.mockResolvedValue("C:\\exports\\settings.json");
    api.previewSettingsImport.mockResolvedValue(preview);
    api.importSettings.mockResolvedValue(undefined);
    render(SettingsFileSection);

    await fireEvent.click(screen.getByRole("button", { name: "Import settings…" }));
    expect(await screen.findByText("2 changes")).toBeTruthy();
    expect(api.previewSettingsImport).toHaveBeenCalledWith("C:\\exports\\settings.json");
    expect(screen.getByText("1 change")).toBeTruthy();
    expect(screen.getByText("No changes")).toBeTruthy();
    expect(screen.getByText('{"l1":"F1"}')).toBeTruthy();

    const prefsBox = screen.getByRole("checkbox", { name: /Preferences/ }) as HTMLInputElement;
    const projectBox = screen.getByRole("checkbox", { name: /Project settings/ }) as HTMLInputElement;
    const uiBox = screen.getByRole("checkbox", { name: /Control & layout/ }) as HTMLInputElement;
    expect(prefsBox.checked).toBe(true);
    expect(uiBox.checked).toBe(true);
    expect(projectBox.disabled).toBe(true);

    await fireEvent.click(uiBox);
    expect(uiBox.checked).toBe(false);

    await fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() =>
      expect(api.importSettings).toHaveBeenCalledWith("C:\\exports\\settings.json", ["prefs"]),
    );
    await waitFor(() => expect(api.getPrefs).toHaveBeenCalled());
    await waitFor(() =>
      expect(toasts.list.some((t) => t.message === "Settings imported. A backup was taken first.")).toBe(
        true,
      ),
    );
    expect(api.currentProject).toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Apply" })).toBeNull();
  });

  it("shows import errors inline", async () => {
    dialog.open.mockResolvedValue("C:\\exports\\settings.json");
    api.previewSettingsImport.mockResolvedValue(preview);
    api.importSettings.mockRejectedValue(err("noProject", "No project is open"));
    render(SettingsFileSection);
    await fireEvent.click(screen.getByRole("button", { name: "Import settings…" }));
    await fireEvent.click(await screen.findByRole("button", { name: "Apply" }));
    expect((await screen.findByRole("alert")).textContent).toContain("Open a project");
    expect(api.getPrefs).not.toHaveBeenCalled();
  });

  it("toasts when the file is not a valid settings file", async () => {
    dialog.open.mockResolvedValue("C:\\x.json");
    api.previewSettingsImport.mockRejectedValue(err("invalid", "Wrong kind."));
    render(SettingsFileSection);
    await fireEvent.click(screen.getByRole("button", { name: "Import settings…" }));
    await waitFor(() => expect(errorToasts()[0]?.message).toContain("isn't a valid settings file"));
  });

  it("exports to the chosen path", async () => {
    dialog.save.mockResolvedValue("C:\\out\\kinetra-settings.json");
    api.exportSettings.mockResolvedValue(undefined);
    render(SettingsFileSection);
    await fireEvent.click(screen.getByRole("button", { name: "Export settings…" }));
    await waitFor(() =>
      expect(api.exportSettings).toHaveBeenCalledWith("C:\\out\\kinetra-settings.json"),
    );
    expect(dialog.save.mock.calls[0][0].defaultPath).toBe("kinetra-settings.json");
  });
});

// ---- Move ---------------------------------------------------------------------------------------

describe("Move project", () => {
  function deferred<T>() {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
      resolve = res;
      reject = rej;
    });
    return { promise, resolve, reject };
  }

  async function openPanel() {
    flow.pickFolder.mockResolvedValue("D:\\new-home");
    render(LocationSection);
    await fireEvent.click(screen.getByRole("button", { name: "Move project…" }));
    await screen.findByRole("heading", { name: "Move project" });
  }

  it("shows progress, cancels, and unsubscribes after a cancelled move", async () => {
    const move = deferred<Project>();
    api.moveProject.mockReturnValue(move.promise);
    api.cancelMove.mockResolvedValue(undefined);
    await openPanel();

    expect(screen.getByText("D:\\new-home")).toBeTruthy();
    const box = screen.getByRole("checkbox", { name: /Delete the original folder/ }) as HTMLInputElement;
    expect(box.checked).toBe(false);
    expect(api.on).not.toHaveBeenCalledWith("project-move-progress", expect.anything());

    await fireEvent.click(screen.getByRole("button", { name: "Move" }));
    expect(api.moveProject).toHaveBeenCalledWith("D:\\new-home", false);
    expect(settingsBusy.moving).toBe(true);
    expect(events.handlers.has("project-move-progress")).toBe(true);

    const progress: MoveProgress = {
      phase: "copying",
      filesDone: 3,
      filesTotal: 10,
      bytesDone: 1.2 * 1024 ** 3,
      bytesTotal: 3.4 * 1024 ** 3,
    };
    events.handlers.get("project-move-progress")!(progress);
    await tick();
    const bar = screen.getByRole("progressbar", { name: "Moving project" });
    expect(bar.getAttribute("aria-valuetext")).toBe("Copying 3 of 10 files · 1.2 of 3.4 GB");
    const line = screen.getByText("Copying 3 of 10 files · 1.2 of 3.4 GB", { selector: "p" });
    expect(line.classList.contains("mono")).toBe(false);

    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(api.cancelMove).toHaveBeenCalled();

    move.reject(err("cancelled", "Canceled"));
    await waitFor(() =>
      expect(toasts.list.some((t) => t.kind === "info" && t.message.startsWith("Move canceled"))).toBe(
        true,
      ),
    );
    expect(events.unsubs.get("project-move-progress")).toHaveBeenCalled();
    expect(settingsBusy.moving).toBe(false);
  });

  it("switches to the moved project and unsubscribes on success", async () => {
    const moved = { ...current, path: "D:\\new-home" };
    api.moveProject.mockResolvedValue(moved);
    await openPanel();
    await fireEvent.click(screen.getByRole("button", { name: "Move" }));
    await waitFor(() => expect(project.current?.path).toBe("D:\\new-home"));
    expect(events.unsubs.get("project-move-progress")).toHaveBeenCalled();
    expect(toasts.list.some((t) => t.kind === "success")).toBe(true);
  });

  it("shows other errors inline and keeps the panel open", async () => {
    api.moveProject.mockRejectedValue(err("notEmpty", "The destination folder isn't empty."));
    await openPanel();
    await fireEvent.click(screen.getByRole("button", { name: "Move" }));
    expect((await screen.findByRole("alert")).textContent).toContain("isn't empty");
    expect(screen.getByRole("heading", { name: "Move project" })).toBeTruthy();
  });
});

// ---- Reset --------------------------------------------------------------------------------------

describe("ResetSection", () => {
  it("resets everything only after confirmation", async () => {
    api.resetSettings.mockResolvedValue(undefined);
    api.currentProject.mockResolvedValue(null);
    render(ResetSection);

    await fireEvent.click(screen.getByRole("button", { name: "Reset everything" }));
    const dlg = await screen.findByRole("dialog");
    expect(within(dlg).getByText(/recent projects list is cleared/)).toBeTruthy();
    expect(api.resetSettings).not.toHaveBeenCalled();

    await fireEvent.click(within(dlg).getByRole("button", { name: "Reset everything" }));
    await waitFor(() => expect(api.resetSettings).toHaveBeenCalledWith("everything"));
    await waitFor(() => expect(api.getPrefs).toHaveBeenCalled());
    await waitFor(() => expect(project.current).toBeNull());
    // The Preferences window stays; the main window follows project-changed.
    expect(nav.goto).not.toHaveBeenCalled();
  });

  it("describes Reset appearance as following the system theme again", async () => {
    render(ResetSection);
    expect(screen.getByText("Follow the system theme again.")).toBeTruthy();
    expect(screen.queryByText(/back to System/)).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Reset appearance" }));
    const dlg = await screen.findByRole("dialog");
    expect(within(dlg).getByText(/follows the system again/)).toBeTruthy();
  });

  it("uses danger buttons only for the two broad resets", () => {
    render(ResetSection);
    const variant = (name: string) => screen.getByRole("button", { name }).classList;
    expect(variant("Reset appearance").contains("danger")).toBe(false);
    expect(variant("Reset control layout").contains("danger")).toBe(false);
    expect(variant("Reset all preferences").contains("danger")).toBe(true);
    expect(variant("Reset everything").contains("danger")).toBe(true);
  });

  it("does nothing when the confirmation is cancelled", async () => {
    render(ResetSection);
    await fireEvent.click(screen.getByRole("button", { name: "Reset appearance" }));
    const dlg = await screen.findByRole("dialog");
    await fireEvent.click(within(dlg).getByRole("button", { name: "Cancel" }));
    await tick();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(api.resetSettings).not.toHaveBeenCalled();
  });
});

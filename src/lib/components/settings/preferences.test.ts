// The Preferences window: the section registry and old-anchor aliases, the /settings redirect, the
// window page (section list, ?section=, prefs-section, deep-link focus), Mode and Zoom, the
// project-only blocks, Simple mode in Network & API, About's update states and the cross-window
// store sync.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Prefs, Project, UiState, UpdateCheck } from "$lib/types";

// ---- Mocks --------------------------------------------------------------------------------------

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Handler>() }));

const api = vi.hoisted(() => ({
  appInfo: vi.fn(),
  openPreferences: vi.fn(async (_section?: string | null) => {}),
  currentProject: vi.fn(),
  updateProjectSettings: vi.fn(),
  getPrefs: vi.fn(),
  setPrefs: vi.fn(),
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  listBackups: vi.fn(async () => []),
  createBackup: vi.fn(),
  resetSettings: vi.fn(),
  serverStatus: vi.fn(),
  on: vi.fn(),
  listScreens: vi.fn(async () => []),
  listFlows: vi.fn(async () => []),
  listSources: vi.fn(async () => []),
  oscStatus: vi.fn(async () => ({ state: "off" })),
  onOscStatus: vi.fn(() => () => {}),
  listMidiDevices: vi.fn(async () => []),
  onMidiDevices: vi.fn(() => () => {}),
  watchMidiDevices: vi.fn(async () => {}),
  testMidiFeedback: vi.fn(async () => {}),
  midiLearn: vi.fn(async () => {}),
  onMidiLearn: vi.fn(() => () => {}),
  onControlsChanged: vi.fn(() => () => {}),
  getControls: vi.fn(async () => ({ schemaVersion: 1, mappings: [] })),
  saveControls: vi.fn(async (d: unknown) => d),
  getScreen: vi.fn(),
  getTemplate: vi.fn(),
  checkForUpdate: vi.fn(),
  installUpdate: vi.fn(),
  setZoom: vi.fn(async (z: number) => z),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e
      ? e
      : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/backups", () => api);
vi.mock("$lib/api/settings-file", () => api);
vi.mock("$lib/api/move", () => api);
vi.mock("$lib/api/reset", () => api);
vi.mock("$lib/api/flows", () => api);
vi.mock("$lib/api/sources", () => api);
vi.mock("$lib/api/midi", () => api);
vi.mock("$lib/api/app", () => api);
vi.mock("$lib/api/controls", () => api);
vi.mock("$lib/api/screens", () => api);
vi.mock("$lib/api/templates", () => api);

const flow = vi.hoisted(() => ({ pickFolder: vi.fn(), revealInExplorer: vi.fn() }));
vi.mock("$lib/projectFlow", () => flow);

const opener = vi.hoisted(() => ({ openUrl: vi.fn(async (_u: string) => {}) }));
vi.mock("@tauri-apps/plugin-opener", () => opener);

const nav = vi.hoisted(() => ({
  goto: vi.fn(async (_url: string) => {}),
  afterNavigate: vi.fn(),
  replaceState: vi.fn(),
}));
vi.mock("$app/navigation", () => nav);
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/preferences") } }));

// Sections load when they come into view: everything is in view here.
class InstantIO {
  constructor(private cb: IntersectionObserverCallback) {}
  observe(target: Element) {
    this.cb([{ target, isIntersecting: true } as unknown as IntersectionObserverEntry], this as unknown as IntersectionObserver);
  }
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
}
vi.stubGlobal("IntersectionObserver", InstantIO);
// jsdom has no scrolling.
if (!Element.prototype.scrollIntoView) Element.prototype.scrollIntoView = function () {};

import PreferencesPage from "../../../routes/preferences/+page.svelte";
import { prefs } from "$lib/stores/prefs.svelte";
import { project } from "$lib/stores/project.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { uiState } from "$lib/stores/uiState.svelte";
import { syncWindows } from "$lib/stores/windows";
import AboutSection from "./AboutSection.svelte";
import GeneralSection from "./GeneralSection.svelte";
import { preferencesLink, redirectSettings } from "./open";
import AdvancedPane from "./panes/AdvancedPane.svelte";
import AppearancePane from "./panes/AppearancePane.svelte";
import NetworkPane from "./panes/NetworkPane.svelte";
import { PREFERENCE_ALIASES, PREFERENCE_ENTRIES, PREFERENCE_SECTIONS, resolveSection } from "./sections";
import { CHECK_FAILED, UP_TO_DATE, updater } from "./updates.svelte";

// ---- Fixtures -----------------------------------------------------------------------------------

const current: Project = {
  path: "C:\\shows\\current",
  name: "Current Show",
  createdAt: "2026-01-02T10:00:00Z",
  settings: { port: 7770, lanAccess: false, defaultResolution: { width: 1920, height: 1080 } },
};
const basePrefs: Prefs = {
  theme: "dark",
  backup: { enabled: true, intervalMinutes: 15, retention: 10 },
  uiMode: "advanced",
};
const baseUi: UiState = {
  sidebarPinned: false,
  control: { view: "grid", gridColumns: 2, selectedScreenId: null, hiddenScreenIds: [], liveEdit: "immediate", hotkeys: {}, automationOff: false },
};
const INFO = {
  version: "0.3.0",
  tauriVersion: "2.8.5",
  webviewVersion: "141.0",
  configDir: "C:\\cfg",
  logDir: "C:\\logs",
  channel: "stable",
  commit: "fa23a08",
};
const check = (patch: Partial<UpdateCheck> = {}): UpdateCheck => ({
  available: true,
  version: "0.4.0",
  notes: "Native menus.\nA Preferences window.",
  date: "2026-10-09T10:00:00Z",
  blocked: null,
  ...patch,
});

beforeEach(() => {
  vi.clearAllMocks();
  events.handlers.clear();
  api.on.mockImplementation((name: string, handler: Handler) => {
    events.handlers.set(name, handler);
    return () => events.handlers.delete(name);
  });
  api.serverStatus.mockResolvedValue({ state: "stopped" });
  api.getPrefs.mockResolvedValue(basePrefs);
  api.setPrefs.mockImplementation(async (p: Prefs) => p);
  api.getUiState.mockResolvedValue(baseUi);
  api.currentProject.mockResolvedValue(current);
  api.appInfo.mockResolvedValue(INFO);
  project.current = current;
  prefs.set(basePrefs);
  uiState.set(baseUi);
  updater.state = { kind: "idle" };
  window.history.replaceState(null, "", "/preferences");
});

afterEach(() => {
  cleanup();
  toasts.clear();
});

const emit = (event: string, payload: unknown) => events.handlers.get(event)?.(payload);

// ---- Registry and aliases -----------------------------------------------------------------------

describe("sections", () => {
  it("are General, Appearance, Outputs, Network & API, License, Shortcuts, Advanced, About, each a lazy body", () => {
    expect(PREFERENCE_SECTIONS.map((s) => s.title)).toEqual(["General", "Appearance", "Outputs", "Network & API", "License", "Shortcuts", "Advanced", "About"]);
    for (const s of PREFERENCE_SECTIONS) expect(typeof s.load).toBe("function");
  });

  it("map old /settings anchors to their section and keep the anchor", () => {
    expect(resolveSection("backups")).toEqual({ section: "advanced", anchor: "backups", check: false });
    expect(resolveSection("#server")).toEqual({ section: "network", anchor: "server", check: false });
    expect(resolveSection("midi")).toEqual({ section: "network", anchor: "midi", check: false });
    expect(resolveSection("style")).toEqual({ section: "appearance", anchor: "style", check: false });
    expect(resolveSection("settings-file").section).toBe("advanced");
    expect(resolveSection("reset").section).toBe("advanced");
    expect(resolveSection("location").section).toBe("advanced");
    // A section id is the section itself (top).
    expect(resolveSection("shortcuts")).toEqual({ section: "shortcuts", anchor: null, check: false });
    expect(resolveSection("about")).toEqual({ section: "about", anchor: null, check: false });
    // Help › Check for updates.
    expect(resolveSection("updates")).toEqual({ section: "about", anchor: null, check: true });
    // Nothing or unknown: General.
    expect(resolveSection(null).section).toBe("general");
    expect(resolveSection("nope").section).toBe("general");
    // Every alias and palette entry lands on a real section.
    const ids = new Set(PREFERENCE_SECTIONS.map((s) => s.id));
    for (const v of Object.values(PREFERENCE_ALIASES)) expect(ids.has(v)).toBe(true);
    for (const e of PREFERENCE_ENTRIES) expect(ids.has(resolveSection(e.id).section)).toBe(true);
  });
});

describe("/settings redirect and links", () => {
  it("opens the window at the hash's section, then goes back (or to the dashboard)", () => {
    const back = vi.fn();
    const replace = vi.fn();
    redirectSettings("#backups", { canGoBack: true, back, replace });
    expect(api.openPreferences).toHaveBeenLastCalledWith("backups");
    expect(back).toHaveBeenCalledTimes(1);
    expect(replace).not.toHaveBeenCalled();

    redirectSettings("", { canGoBack: false, back, replace });
    expect(api.openPreferences).toHaveBeenLastCalledWith(undefined);
    expect(replace).toHaveBeenCalledWith("/dashboard");
  });

  it("a Preferences link opens the window instead of navigating", () => {
    const after = vi.fn();
    const e = new MouseEvent("click", { cancelable: true });
    preferencesLink("midi", after)(e);
    expect(e.defaultPrevented).toBe(true);
    expect(after).toHaveBeenCalled();
    expect(api.openPreferences).toHaveBeenCalledWith("midi");
  });

  it("a failure to open the window is a toast", async () => {
    api.openPreferences.mockRejectedValueOnce({ code: "internal", message: "no window" });
    preferencesLink("server")(new MouseEvent("click", { cancelable: true }));
    await waitFor(() => expect(toasts.list.at(-1)?.message).toBe("Couldn't open Preferences: no window"));
  });
});

// ---- The window ---------------------------------------------------------------------------------

describe("Preferences window", () => {
  const list = () => screen.getByRole("navigation", { name: "Preferences sections" });
  const active = () => within(list()).getAllByRole("button").find((b) => b.getAttribute("aria-current") === "page");

  it("lists the sections and opens General; restores the project; clicking switches", async () => {
    project.current = null;
    render(PreferencesPage);
    expect(within(list()).getAllByRole("button").map((b) => b.textContent?.trim())).toEqual([
      "General",
      "Appearance",
      "Outputs",
      "Network & API",
      "License",
      "Shortcuts",
      "Advanced",
      "About",
    ]);
    expect(active()?.textContent?.trim()).toBe("General");
    expect(active()?.querySelector(".dot")).toBeTruthy();
    await screen.findByRole("radiogroup", { name: "Mode" });
    await waitFor(() => expect(project.current).toEqual(current));
    expect(api.currentProject).toHaveBeenCalled();

    await fireEvent.click(within(list()).getByRole("button", { name: "Appearance" }));
    await screen.findByRole("radiogroup", { name: "Theme" });
    expect(active()?.textContent?.trim()).toBe("Appearance");
    expect(nav.goto).toHaveBeenLastCalledWith("?section=appearance", { replaceState: true, noScroll: true, keepFocus: true });
  });

  it("arrow keys move through the list and show the section", async () => {
    render(PreferencesPage);
    const first = within(list()).getByRole("button", { name: "General" });
    first.focus();
    await fireEvent.keyDown(first, { key: "ArrowDown" });
    expect(document.activeElement?.textContent?.trim()).toBe("Appearance");
    expect(active()?.textContent?.trim()).toBe("Appearance");
    await fireEvent.keyDown(document.activeElement!, { key: "End" });
    expect(active()?.textContent?.trim()).toBe("About");
  });

  it("opens ?section=backups at Advanced, scrolled to Backups with focus there", async () => {
    const scrolled: string[] = [];
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = function (this: Element) {
      scrolled.push(this.id);
    };
    try {
      window.history.replaceState(null, "", "/preferences?section=backups");
      render(PreferencesPage);
      await waitFor(() => expect(document.activeElement?.id).toBe("backups"));
      expect(active()?.textContent?.trim()).toBe("Advanced");
      expect(scrolled).toContain("backups");
      expect(screen.getByRole("heading", { name: "Backups" })).toBeTruthy();
    } finally {
      Element.prototype.scrollIntoView = original;
    }
  });

  it("follows prefs-section while open (old anchors too)", async () => {
    render(PreferencesPage);
    await screen.findByRole("radiogroup", { name: "Mode" });
    emit("prefs-section", { section: "server" });
    await waitFor(() => expect(active()?.textContent?.trim()).toBe("Network & API"));
    await waitFor(() => expect(document.activeElement?.id).toBe("server"));
  });

  it("prefs-section updates opens About and checks for updates", async () => {
    api.checkForUpdate.mockResolvedValue(check({ available: false, version: null }));
    render(PreferencesPage);
    emit("prefs-section", { section: "updates" });
    await waitFor(() => expect(active()?.textContent?.trim()).toBe("About"));
    await screen.findByText(UP_TO_DATE);
    expect(api.checkForUpdate).toHaveBeenCalledTimes(1);
  });

  it("unsubscribes from prefs-section on close", () => {
    const { unmount } = render(PreferencesPage);
    expect(events.handlers.has("prefs-section")).toBe(true);
    unmount();
    expect(events.handlers.has("prefs-section")).toBe(false);
  });
});

// ---- General / Appearance -----------------------------------------------------------------------

describe("General", () => {
  it("switches Mode through prefs (per machine)", async () => {
    render(GeneralSection);
    const group = screen.getByRole("radiogroup", { name: "Mode" });
    expect(within(group).getByRole("radio", { name: "Advanced" }).getAttribute("aria-checked")).toBe("true");
    await fireEvent.click(within(group).getByRole("radio", { name: "Simple" }));
    expect(api.setPrefs).toHaveBeenCalledWith({ ...basePrefs, uiMode: "simple" });
  });

  it("shows one line instead of the project's settings without a project", () => {
    project.current = null;
    render(GeneralSection);
    expect(screen.getByText("Open a project to change its settings.")).toBeTruthy();
    expect(screen.queryByLabelText("Default screen resolution")).toBeNull();
    expect(screen.getByRole("radiogroup", { name: "Mode" })).toBeTruthy();
  });
});

describe("Appearance", () => {
  it("sets the theme and the zoom", async () => {
    render(AppearancePane);
    await fireEvent.click(screen.getByRole("radio", { name: "Light" }));
    await waitFor(() => expect(api.setPrefs).toHaveBeenCalledWith(expect.objectContaining({ theme: "light" })));

    const zoom = screen.getByLabelText("Zoom") as HTMLInputElement;
    expect(zoom.value).toBe("100");
    await fireEvent.change(zoom, { target: { value: "125" } });
    // Snapped to 10 % steps (120 or 130), clamped 50–200.
    await waitFor(() => expect(api.setZoom).toHaveBeenCalledTimes(1));
    const sent = api.setZoom.mock.calls[0][0];
    expect([1.2, 1.3]).toContain(sent);
    await waitFor(() => expect(prefs.value?.zoom).toBe(sent));
    await fireEvent.click(screen.getByRole("button", { name: "Actual size" }));
    expect(api.setZoom).toHaveBeenLastCalledWith(1);
  });

  it("keeps Style & brand behind an open project", () => {
    project.current = null;
    render(AppearancePane);
    expect(screen.getByRole("heading", { name: "Style & brand" })).toBeTruthy();
    expect(screen.getByText("Open a project to change its settings.")).toBeTruthy();
  });
});

// ---- Network & API / Advanced -------------------------------------------------------------------

describe("Network & API", () => {
  it("Advanced: Server with OSC and MIDI & OSC; anchors server, osc and midi", async () => {
    const { container } = render(NetworkPane);
    expect(screen.getByRole("heading", { name: "Server" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "MIDI & OSC" })).toBeTruthy();
    expect(container.querySelector("#server")).toBeTruthy();
    expect(container.querySelector("#midi")).toBeTruthy();
    await waitFor(() => expect(container.querySelector("#osc")).toBeTruthy());
    expect(screen.queryByText(/available in Advanced mode/)).toBeNull();
  });

  it("Simple: one hint instead of MIDI and OSC; switching shows them", async () => {
    prefs.set({ ...basePrefs, uiMode: "simple" });
    const { container } = render(NetworkPane);
    expect(screen.getByRole("heading", { name: "Server" })).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "MIDI & OSC" })).toBeNull();
    await tick();
    expect(container.querySelector("#osc")).toBeNull();
    expect(screen.getByText("MIDI and OSC: available in Advanced mode", { exact: false })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Switch to Advanced" }));
    await waitFor(() => expect(screen.getByRole("heading", { name: "MIDI & OSC" })).toBeTruthy());
    expect(api.setPrefs).toHaveBeenCalledWith({ ...basePrefs, uiMode: "advanced" });
  });
});

describe("Advanced", () => {
  it("keeps the old anchors; Location and Backups need a project, Settings file and Reset don't", () => {
    project.current = null;
    const { container } = render(AdvancedPane);
    expect([...container.querySelectorAll(".anchor")].map((a) => a.id)).toEqual(["location", "backups", "settings-file", "reset"]);
    expect(screen.getAllByText("Open a project to change its settings.")).toHaveLength(2);
    expect(screen.getByRole("button", { name: "Export settings…" })).toBeTruthy();
    // Without a project, the project's control layout can't be reset.
    expect(screen.queryByRole("button", { name: "Reset control layout" })).toBeNull();
    expect(screen.getByRole("button", { name: "Reset everything" })).toBeTruthy();
  });
});

// ---- About: updates -----------------------------------------------------------------------------

describe("About: updates", () => {
  it("shows version, channel and commit, and the links open in the browser", async () => {
    render(AboutSection);
    await screen.findByText("0.3.0");
    expect(screen.getByText("Stable")).toBeTruthy();
    expect(screen.getByText("fa23a08").classList.contains("mono")).toBe(true);
    await fireEvent.click(screen.getByRole("link", { name: /kinetra\.live\/docs/ }));
    await waitFor(() => expect(opener.openUrl).toHaveBeenCalledWith("https://kinetra.live/docs"));
    await fireEvent.click(screen.getByRole("link", { name: /^kinetra\.live$/ }));
    await waitFor(() => expect(opener.openUrl).toHaveBeenCalledWith("https://kinetra.live"));
    await fireEvent.click(screen.getByRole("button", { name: "Join our Discord" }));
    await waitFor(() => expect(opener.openUrl).toHaveBeenCalledWith("https://discord.gg/nK4JaEh3"));
  });

  it("up to date", async () => {
    api.checkForUpdate.mockResolvedValue(check({ available: false, version: null, notes: null }));
    render(AboutSection);
    await fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
    expect(await screen.findByText(UP_TO_DATE)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Install and restart" })).toBeNull();
  });

  it("a failed check says so, whatever the error", async () => {
    api.checkForUpdate.mockRejectedValue({ code: "updateCheck", message: "dns error: no such host" });
    render(AboutSection);
    await fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
    expect(await screen.findByText(CHECK_FAILED)).toBeTruthy();
    expect(CHECK_FAILED).toBe("Couldn't check for updates. Try again later.");
    expect(screen.queryByText(/dns error/)).toBeNull();
  });

  it("a new version with notes; Install is disabled with the reason while on air", async () => {
    api.checkForUpdate.mockResolvedValue(check({ blocked: "Take everything off air before installing." }));
    render(AboutSection);
    await fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
    await screen.findByText(/is available/);
    expect(screen.getByRole("status").textContent?.replace(/\s+/g, " ").trim()).toBe("Version 0.4.0 is available");
    expect(screen.getByText(/Native menus\./)).toBeTruthy();
    const install = screen.getByRole("button", { name: "Install and restart" }) as HTMLButtonElement;
    expect(install.disabled).toBe(true);
    expect(install.getAttribute("aria-describedby")).toBe("pref-update-blocked");
    expect(screen.getByText("Take everything off air before installing.")).toBeTruthy();
  });

  it("installs with progress; an on-air refusal disables Install with its reason", async () => {
    api.checkForUpdate.mockResolvedValue(check());
    let fail: (e: unknown) => void = () => {};
    api.installUpdate.mockImplementation(() => new Promise((_, reject) => (fail = reject)));
    render(AboutSection);
    await fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
    await fireEvent.click(await screen.findByRole("button", { name: "Install and restart" }));
    expect(api.installUpdate).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(events.handlers.has("update-progress")).toBe(true));
    emit("update-progress", { downloaded: 5 * 1024 * 1024, total: 20 * 1024 * 1024, phase: "download" });
    const bar = await screen.findByRole("progressbar", { name: "Installing the update" });
    await waitFor(() => expect(bar.getAttribute("aria-valuenow")).toBe(String(5 * 1024 * 1024)));
    expect(screen.getByText(/^Downloading /)).toBeTruthy();
    expect((screen.getByRole("button", { name: "Check for updates" }) as HTMLButtonElement).disabled).toBe(true);

    fail({ code: "onAir", message: "Take everything off air before installing." });
    await screen.findByText("Take everything off air before installing.");
    expect((screen.getByRole("button", { name: "Install and restart" }) as HTMLButtonElement).disabled).toBe(true);
    expect(events.handlers.has("update-progress")).toBe(false);
  });

  it("other install errors show under the button", async () => {
    api.checkForUpdate.mockResolvedValue(check());
    api.installUpdate.mockRejectedValue({ code: "internal", message: "signature mismatch" });
    render(AboutSection);
    await fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
    await fireEvent.click(await screen.findByRole("button", { name: "Install and restart" }));
    expect((await screen.findByRole("alert")).textContent).toBe("Couldn't install the update: signature mismatch");
  });
});

// ---- Cross-window sync --------------------------------------------------------------------------

describe("syncWindows", () => {
  it("applies prefs-changed, ui-state-changed and project-changed; unsubscribes", async () => {
    const stop = syncWindows();
    emit("prefs-changed", { ...basePrefs, theme: "light", zoom: 1.5 });
    expect(prefs.value?.zoom).toBe(1.5);
    emit("ui-state-changed", { ...baseUi, sidebarPinned: true });
    expect(uiState.value?.sidebarPinned).toBe(true);

    // A moved project (another path) reloads its UI state.
    emit("project-changed", { ...current, path: "D:\\moved" });
    expect(project.current?.path).toBe("D:\\moved");
    await waitFor(() => expect(api.getUiState).toHaveBeenCalled());

    // Closed elsewhere: no project, no UI state.
    emit("project-changed", null);
    expect(project.current).toBeNull();
    expect(uiState.value).toBeNull();

    stop();
    expect(events.handlers.has("prefs-changed")).toBe(false);
    expect(events.handlers.has("ui-state-changed")).toBe(false);
    expect(events.handlers.has("project-changed")).toBe(false);
  });
});

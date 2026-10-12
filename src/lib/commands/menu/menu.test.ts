// The native menu: the spec built from the registry (structure, keys from bindings, enabled and
// checked states, Windows vs macOS, cues and recents), the sync (sent once per change) and menu
// clicks running commands.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushSync } from "svelte";
import type { MenuItem, MenuNode, MenuSpec, MenuSubmenu, OutputConfig, OutputStatus, Prefs, RecentProject, UiState } from "$lib/types";

const api = vi.hoisted(() => {
  const handlers = new Map<string, (p: unknown) => void>();
  return {
    handlers,
    getPrefs: vi.fn(),
    setPrefs: vi.fn(async (p: unknown) => p),
    getUiState: vi.fn(),
    setUiState: vi.fn(async (u: unknown) => u),
    openPreferences: vi.fn(async (_section?: string) => {}),
    listRecents: vi.fn(async () => [] as RecentProject[]),
    clearRecents: vi.fn(async () => {}),
    on: vi.fn((event: string, h: (p: unknown) => void) => {
      handlers.set(event, h);
      return () => handlers.delete(event);
    }),
  };
});
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) => ({ code: "internal", message: e instanceof Error ? e.message : String(e) }),
}));
const app = vi.hoisted(() => ({
  setAppMenu: vi.fn(async (_spec: unknown) => {}),
  setZoom: vi.fn(async (z: number) => z),
  toggleFullscreen: vi.fn(async () => true),
  exitApp: vi.fn(async () => {}),
  closeRequestAck: vi.fn(async () => {}),
}));
vi.mock("$lib/api/app", () => app);
vi.mock("$lib/api/live", () => ({ listScreens: vi.fn(async () => []) }));
const outputsApi = vi.hoisted(() => ({
  listOutputs: vi.fn(async () => [] as OutputStatus[]),
  allOutputsOff: vi.fn(async () => {}),
}));
vi.mock("$lib/api/outputs", () => outputsApi);
const dialog = vi.hoisted(() => ({ ask: vi.fn(async (_m: string, _o?: unknown) => true) }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);
vi.mock("$app/navigation", () => ({ goto: vi.fn(async () => {}) }));

import { prefs } from "$lib/stores/prefs.svelte";
import { project, uiState } from "$lib/stores/project.svelte";
import { setKeys } from "../bindings.svelte";
import { provideContext, registerCommands, registerProvider, type Command } from "../registry.svelte";
import { appCommands, clampZoom } from "./commands";
import { buildMenuSpec, escapeMenuText } from "./spec";
import { menuInput, onCloseRequested, runMenuCommand, startMenuSync } from "./sync.svelte";

const basePrefs = (patch: Partial<Prefs> = {}): Prefs => ({
  theme: "dark",
  backup: { enabled: true, intervalMinutes: 15, retention: 10 },
  uiMode: "advanced",
  ...patch,
});
const ui = (patch: Partial<UiState> = {}): UiState => ({
  sidebarPinned: true,
  control: {
    view: "grid",
    gridColumns: 2,
    selectedScreenId: null,
    hiddenScreenIds: [],
    liveEdit: "onTake",
    hotkeys: {},
    automationOff: false,
  },
  ...patch,
});
const cmd = (id: string, patch: Partial<Command> = {}): Command => ({ id, title: id, section: "settings", run: vi.fn(), ...patch });

let cleanups: (() => void)[] = [];
const keep = (off: () => void) => (cleanups.push(off), off);

beforeEach(() => {
  vi.clearAllMocks();
  prefs.set(basePrefs());
  uiState.set(ui());
  project.current = null;
  project.recents = [];
  keep(registerCommands(appCommands()));
});
afterEach(() => {
  for (const off of cleanups.reverse()) off();
  cleanups = [];
});

const menu = (spec: MenuSpec, id: string): MenuSubmenu => {
  const find = (nodes: readonly MenuNode[]): MenuSubmenu | undefined => {
    for (const n of nodes) {
      if (n.kind !== "submenu") continue;
      if (n.id === id) return n;
      const inner = find(n.items);
      if (inner) return inner;
    }
    return undefined;
  };
  const m = find(spec.menus);
  if (!m) throw new Error(`no ${id}`);
  return m;
};
const items = (m: MenuSubmenu) => m.items.filter((n): n is MenuItem => n.kind === "item");
const itemOf = (spec: MenuSpec, id: string): MenuItem => {
  const all: MenuItem[] = [];
  const walk = (nodes: readonly MenuNode[]) => {
    for (const n of nodes) {
      if (n.kind === "item") all.push(n);
      else if (n.kind === "submenu") walk(n.items);
    }
  };
  walk(spec.menus);
  const it = all.find((i) => i.id === id);
  if (!it) throw new Error(`no ${id}`);
  return it;
};
const shape = (nodes: readonly MenuNode[]): unknown[] =>
  nodes.map((n) => (n.kind === "item" ? n.id : n.kind === "submenu" ? { [n.id]: shape(n.items) } : n.kind === "predefined" ? `<${n.item}>` : "---"));

describe("buildMenuSpec", () => {
  it("builds File, Edit, View, Show, Output and Help (Windows)", () => {
    const spec = buildMenuSpec(menuInput(false));
    expect(spec.menus.map((m) => m.text)).toEqual(["&File", "&Edit", "&View", "&Show", "&Output", "&Help"]);
    expect(shape(spec.menus)).toEqual([
      {
        "menu.file": [
          "app.newProject",
          "app.openProject",
          { "menu.file.recent": ["app.clearRecents"] },
          "---",
          { "menu.file.import": ["app.importSettings"] },
          { "menu.file.export": ["app.exportSettings"] },
          "---",
          "app.preferences",
          "---",
          "app.exit",
        ],
      },
      { "menu.edit": ["edit.undo", "edit.redo", "---", "<cut>", "<copy>", "<paste>", "---", "<selectAll>"] },
      {
        "menu.view": [
          "view.simple",
          "view.advanced",
          "---",
          { "menu.view.panels": ["shell.pin"] },
          "---",
          "view.zoomIn",
          "view.zoomOut",
          "view.actualSize",
          "---",
          "view.fullscreen",
        ],
      },
      { "menu.show": ["show.take", "show.clear", "---", "live.clearAll", "live.panic", "---", { "menu.show.go": [] }] },
      { "menu.output": ["output.displays"] },
      { "menu.help": ["help.docs", "help.shortcuts", "help.updates", "---", "help.about"] },
    ]);
  });

  it("macOS: predefined Undo / Redo", () => {
    const spec = buildMenuSpec(menuInput(true));
    expect(shape(menu(spec, "menu.edit").items)).toEqual(["<undo>", "<redo>", "---", "<cut>", "<copy>", "<paste>", "---", "<selectAll>"]);
  });

  it("shows the user's keys (first binding), fixed Undo / Redo keys and Preferences with Ctrl+,", async () => {
    keep(registerCommands([cmd("nav.settings", { defaultKeys: ["Ctrl+,"] })]));
    let spec = buildMenuSpec(menuInput(false));
    expect(itemOf(spec, "view.zoomIn").keys).toBe("Ctrl+=");
    expect(itemOf(spec, "view.fullscreen").keys).toBe("F11");
    expect(itemOf(spec, "edit.undo").keys).toBe("Ctrl+Z");
    expect(itemOf(spec, "edit.redo").keys).toBe("Ctrl+Y");
    expect(itemOf(spec, "app.preferences").keys).toBe("Ctrl+,");
    expect(itemOf(spec, "app.exit").keys).toBeNull();
    await setKeys("view.zoomIn", ["Ctrl+Shift+=", "Ctrl+I"]);
    spec = buildMenuSpec(menuInput(false));
    expect(itemOf(spec, "view.zoomIn").keys).toBe("Ctrl+Shift+=");
  });

  it("enabled = registered, when and not disabled; texts are translated menu texts", () => {
    let spec = buildMenuSpec(menuInput(false));
    // Not registered outside Control / the shell.
    expect(itemOf(spec, "live.panic").enabled).toBe(false);
    expect(itemOf(spec, "shell.pin")).toMatchObject({ text: "Sidebar", enabled: false, checked: true });
    // No project: settings import / export are off.
    expect(itemOf(spec, "app.importSettings").enabled).toBe(false);
    expect(itemOf(spec, "app.newProject")).toMatchObject({ text: "New project…", enabled: true });
    keep(registerCommands([cmd("live.panic", { disabled: () => "busy" }), cmd("shell.pin")]));
    keep(provideContext({ project: true }));
    spec = buildMenuSpec(menuInput(false));
    expect(itemOf(spec, "live.panic").enabled).toBe(false);
    expect(itemOf(spec, "shell.pin").enabled).toBe(true);
    expect(itemOf(spec, "app.importSettings").enabled).toBe(true);
  });

  it("checks the current mode", () => {
    prefs.set(basePrefs({ uiMode: "simple" }));
    const spec = buildMenuSpec(menuInput(false));
    expect(itemOf(spec, "view.simple").checked).toBe(true);
    expect(itemOf(spec, "view.advanced").checked).toBe(false);
  });

  it("Show › Go lists the cues with their keys; Take / Clear follow Control's selected layer", () => {
    keep(provideContext({ route: "control", project: true, screenId: "s1", layerIds: [] }));
    keep(
      registerCommands([
        cmd("cue.go:f1", { title: "GO {cue}", params: () => ({ cue: "Intro & outro" }), defaultKeys: ["F5"] }),
        cmd("cue.go:f2", { title: "GO {cue}", params: () => ({ cue: "Break" }), when: () => false }),
      ]),
    );
    let spec = buildMenuSpec(menuInput(false));
    const go = menu(spec, "menu.show.go");
    expect(go.enabled).toBe(true);
    expect(items(go)).toEqual([
      { kind: "item", id: "cue.go:f1", text: "GO Intro && outro", keys: "F5", enabled: true },
      { kind: "item", id: "cue.go:f2", text: "GO Break", keys: null, enabled: false },
    ]);
    // No layer selected.
    expect(itemOf(spec, "show.take").enabled).toBe(false);
    keep(provideContext({ layerIds: ["l1"] }));
    keep(registerProvider({ id: "control.live", items: () => [cmd("live.take:s1/l1")] }));
    spec = buildMenuSpec(menuInput(false));
    expect(itemOf(spec, "show.take").enabled).toBe(true);
    expect(itemOf(spec, "show.clear").enabled).toBe(false);
  });

  it("File › Open recent lists the other recent projects, newest first; missing ones disabled", () => {
    project.current = { path: "C:\\a", name: "A", createdAt: "", settings: { port: 7770, lanAccess: false, defaultResolution: { width: 1920, height: 1080 } } };
    project.recents = [
      { path: "C:\\a", name: "A", lastOpenedAt: "", missing: false },
      { path: "C:\\b", name: "R&D", lastOpenedAt: "", missing: false },
      { path: "C:\\c", name: "C", lastOpenedAt: "", missing: true },
    ];
    const spec = buildMenuSpec(menuInput(false));
    const recent = menu(spec, "menu.file.recent");
    expect(recent.enabled).toBe(true);
    expect(shape(recent.items)).toEqual(["app.openRecent:C:\\b", "app.openRecent:C:\\c", "---", "app.clearRecents"]);
    expect(items(recent).map((i) => [i.text, i.enabled])).toEqual([
      ["Open R&&D", true],
      ["Open C", false],
      ["Clear recent", true],
    ]);
  });

  it("without recents the submenu is disabled", () => {
    expect(menu(buildMenuSpec(menuInput(false)), "menu.file.recent").enabled).toBe(false);
  });

  it("escapes ampersands and clamps zoom", () => {
    expect(escapeMenuText("Style & brand")).toBe("Style && brand");
    expect(clampZoom(2.3)).toBe(2);
    expect(clampZoom(0.2)).toBe(0.5);
    expect(clampZoom(1.1000000001)).toBe(1.1);
  });
});

const output = (id: string, name: string, enabled: boolean): OutputConfig => ({
  id,
  kind: "display",
  name,
  enabled,
  display: { id: `d-${id}`, name: "DELL U2720Q", edidKey: "", width: 2560, height: 1440 },
  screenId: "s1",
  mode: "fill",
  background: { kind: "black" },
  keyOf: null,
  alwaysOnTop: true,
});

describe("Output menu", () => {
  it("lists one checkable item per output, then All outputs off", () => {
    prefs.set(basePrefs({ outputs: [output("o1", "Output 1", true), output("o2", "Output & Co", false)] }));
    const spec = buildMenuSpec(menuInput(false));
    const m = menu(spec, "menu.output");
    expect(shape(m.items)).toEqual(["output.displays", "---", "output.toggle:o1", "output.toggle:o2", "---", "output.allOff"]);
    expect(itemOf(spec, "output.displays").text).toBe("Displays…");
    expect(itemOf(spec, "output.toggle:o1")).toMatchObject({ text: "Output 1 — DELL U2720Q", checked: true, enabled: true });
    expect(itemOf(spec, "output.toggle:o2")).toMatchObject({ text: "Output && Co — DELL U2720Q", checked: false });
    expect(itemOf(spec, "output.allOff").enabled).toBe(true);
    prefs.set(basePrefs({ outputs: [output("o2", "Output 2", false)] }));
    expect(itemOf(buildMenuSpec(menuInput(false)), "output.allOff").enabled).toBe(false);
  });

  it("toggles an output, turns all off, opens Displays…", async () => {
    keep(registerProvider((await import("./commands")).outputsProvider));
    prefs.set(basePrefs({ outputs: [output("o1", "Output 1", false)] }));
    await runMenuCommand("output.toggle:o1");
    expect(api.setPrefs).toHaveBeenLastCalledWith(expect.objectContaining({ outputs: [expect.objectContaining({ id: "o1", enabled: true })] }));
    expect(prefs.value?.outputs?.[0].enabled).toBe(true);
    api.getPrefs.mockResolvedValueOnce(basePrefs({ outputs: [output("o1", "Output 1", false)] }));
    await runMenuCommand("output.allOff");
    expect(outputsApi.allOutputsOff).toHaveBeenCalled();
    expect(prefs.value?.outputs?.[0].enabled).toBe(false);
    await runMenuCommand("output.displays");
    expect(api.openPreferences).toHaveBeenCalledWith("outputs");
  });

  it("Exit asks while outputs are showing", async () => {
    prefs.set(basePrefs({ outputs: [output("o1", "Output 1", true), output("o2", "Output 2", true)] }));
    const st = (id: string): OutputStatus => ({ id, state: "showing", displayId: null, frameLocked: null, pattern: null, message: null });
    outputsApi.listOutputs.mockResolvedValueOnce([st("o1"), st("o2")]);
    dialog.ask.mockResolvedValueOnce(false);
    await runMenuCommand("app.exit");
    expect(dialog.ask).toHaveBeenCalledWith("2 outputs are showing. Exit anyway?", expect.anything());
    expect(app.exitApp).not.toHaveBeenCalled();
  });
});

describe("menu clicks and the sync", () => {
  it("runs the command through the registry's rules, without recording Recent", async () => {
    const run = vi.fn();
    keep(registerCommands([cmd("x.ok", { run }), cmd("x.off", { run, disabled: () => "no" }), cmd("x.never", { run, when: () => false })]));
    expect(await runMenuCommand("x.ok")).toBe(true);
    expect(await runMenuCommand("x.off")).toBe(false);
    expect(await runMenuCommand("x.never")).toBe(false);
    expect(await runMenuCommand("nope")).toBe(false);
    expect(run).toHaveBeenCalledTimes(1);
    expect(api.setUiState).not.toHaveBeenCalled();
  });

  it("runs confirm commands (Clear all, Panic) at once, like their keys", async () => {
    const run = vi.fn();
    keep(registerCommands([cmd("live.panic", { run, confirm: true, live: true })]));
    expect(await runMenuCommand("live.panic")).toBe(true);
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("Help and View items call the app commands", async () => {
    await runMenuCommand("help.about");
    expect(api.openPreferences).toHaveBeenCalledWith("about");
    await runMenuCommand("help.updates");
    expect(api.openPreferences).toHaveBeenCalledWith("updates");
    await runMenuCommand("help.shortcuts");
    expect(api.openPreferences).toHaveBeenCalledWith("shortcuts");
    await runMenuCommand("view.zoomIn");
    expect(app.setZoom).toHaveBeenCalledWith(1.1);
    expect(prefs.value?.zoom).toBe(1.1);
    await runMenuCommand("view.actualSize");
    expect(app.setZoom).toHaveBeenLastCalledWith(1);
    expect(prefs.value?.zoom).toBeUndefined();
    await runMenuCommand("view.simple");
    expect(prefs.value?.uiMode).toBe("simple");
    await runMenuCommand("view.fullscreen");
    expect(app.toggleFullscreen).toHaveBeenCalled();
    await runMenuCommand("app.exit");
    expect(app.exitApp).toHaveBeenCalled();
  });

  it("sends the spec once, again only when it changed, and runs `menu` clicks", async () => {
    const send = vi.fn(async (_s: MenuSpec) => {});
    const stop = startMenuSync(send);
    keep(stop);
    flushSync();
    expect(send).toHaveBeenCalledTimes(1);
    // A context change that doesn't change the menu sends nothing.
    keep(provideContext({ screenId: "zzz" }));
    flushSync();
    expect(send).toHaveBeenCalledTimes(1);
    // A mode change does.
    prefs.set(basePrefs({ uiMode: "simple" }));
    flushSync();
    expect(send).toHaveBeenCalledTimes(2);
    expect(itemOf(send.mock.calls[1][0], "view.simple").checked).toBe(true);
    // A click.
    api.handlers.get("menu")?.({ id: "help.docs.x" });
    api.handlers.get("menu")?.({ id: "help.about" });
    await vi.waitFor(() => expect(api.openPreferences).toHaveBeenCalledWith("about"));
  });
});

describe("the window's close button", () => {
  it("answers, then exits at once when nothing is on air and no output is showing", async () => {
    keep(startMenuSync(vi.fn(async () => {})));
    api.handlers.get("close-requested")?.(null);
    await vi.waitFor(() => expect(app.exitApp).toHaveBeenCalledTimes(1));
    expect(app.closeRequestAck).toHaveBeenCalledTimes(1);
    expect(app.closeRequestAck.mock.invocationCallOrder[0]).toBeLessThan(app.exitApp.mock.invocationCallOrder[0]);
    expect(dialog.ask).not.toHaveBeenCalled();
  });

  it("asks like File › Exit while outputs are showing; Cancel keeps Kinetra open", async () => {
    prefs.set(basePrefs({ outputs: [output("o1", "Output 1", true)] }));
    const st: OutputStatus = { id: "o1", state: "showing", displayId: null, frameLocked: null, pattern: null, message: null };
    outputsApi.listOutputs.mockResolvedValue([st]);
    let answer!: (ok: boolean) => void;
    dialog.ask.mockImplementationOnce(() => new Promise<boolean>((r) => (answer = r)));
    const first = onCloseRequested();
    await vi.waitFor(() => expect(dialog.ask).toHaveBeenCalledTimes(1));
    expect(dialog.ask.mock.calls[0][0]).toBe("1 output is showing. Exit anyway?");
    // Another close while the question is up: answered, no second question.
    await onCloseRequested();
    expect(app.closeRequestAck).toHaveBeenCalledTimes(2);
    expect(dialog.ask).toHaveBeenCalledTimes(1);
    answer(false);
    await first;
    expect(app.exitApp).not.toHaveBeenCalled();
    // Asked again next time, and Exit exits.
    dialog.ask.mockResolvedValueOnce(true);
    await onCloseRequested();
    expect(dialog.ask).toHaveBeenCalledTimes(2);
    expect(app.exitApp).toHaveBeenCalledTimes(1);
    outputsApi.listOutputs.mockReset();
    outputsApi.listOutputs.mockResolvedValue([]);
  });

  it("still runs File › Exit when the answer fails", async () => {
    app.closeRequestAck.mockRejectedValueOnce(new Error("gone"));
    await onCloseRequested();
    expect(app.exitApp).toHaveBeenCalledTimes(1);
  });
});

describe("default keys", () => {
  it("the app commands' defaults conflict with no page's or the shell's", async () => {
    const { pageCommands, inert } = await import("$lib/components/settings/shortcuts/catalog");
    const { conflicts } = await import("../conflicts");
    const own = new Set(appCommands().map((c) => c.id));
    keep(registerCommands(pageCommands().filter((c) => !own.has(c.id)).map(inert)));
    expect(conflicts()).toEqual([]);
  });
});

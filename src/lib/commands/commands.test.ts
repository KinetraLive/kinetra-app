// The command system's shell half: registry (context, running, Recent, follow-ups), the binding
// table (defaults, machine and project tables, conflicts), key matching and t().
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Prefs, UiState } from "$lib/types";

const api = vi.hoisted(() => ({
  getPrefs: vi.fn(),
  setPrefs: vi.fn(async (p: unknown) => p),
  getUiState: vi.fn(),
  setUiState: vi.fn(async (u: unknown) => u),
  openPreferences: vi.fn(async (_section?: string) => {}),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) => ({ code: "internal", message: e instanceof Error ? e.message : String(e) }),
}));
const nav = vi.hoisted(() => ({ goto: vi.fn(async (_url: string) => {}) }));
vi.mock("$app/navigation", () => nav);

import { setMessages, t } from "$lib/i18n";
import { prefs } from "$lib/stores/prefs.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { uiState } from "$lib/stores/uiState.svelte";
import { bindingOf, keysFor, setKeys } from "./bindings.svelte";
import { conflicts, projectBindings } from "./conflicts";
import { eventKeys, normalizeKeys } from "./keys";
import {
  commands,
  currentContext,
  getCommand,
  provideContext,
  recentIds,
  registerCommands,
  registerProvider,
  routeOf,
  runCommand,
  runFollowUp,
  runShortcut,
  setFollowUpOpener,
  type Command,
} from "./registry.svelte";
import { installShortcuts } from "./testing";

const ui = (patch: Partial<UiState["control"]> = {}, recent: string[] = []): UiState => ({
  sidebarPinned: false,
  control: {
    view: "grid",
    gridColumns: 2,
    selectedScreenId: null,
    hiddenScreenIds: [],
    liveEdit: "onTake",
    hotkeys: {},
    automationOff: false,
    ...patch,
  },
  palette: { recent },
});
const basePrefs = (keybindings: Record<string, string[]> = {}): Prefs => ({
  theme: "dark",
  backup: { enabled: true, intervalMinutes: 15, retention: 10 },
  keybindings,
});

const key = (init: KeyboardEventInit & { target?: EventTarget }) => {
  const { target, ...rest } = init;
  const e = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...rest });
  if (target) Object.defineProperty(e, "target", { value: target });
  return e;
};
const cmd = (id: string, patch: Partial<Command> = {}): Command => ({ id, title: id, section: "settings", run: vi.fn(), ...patch });

let cleanups: (() => void)[] = [];
const keep = (off: () => void) => {
  cleanups.push(off);
  return off;
};

beforeEach(() => {
  vi.clearAllMocks();
  prefs.set(basePrefs());
  uiState.set(ui());
});

afterEach(() => {
  for (const off of cleanups.reverse()) off();
  cleanups = [];
  toasts.clear();
  setMessages(null);
  document.body.innerHTML = "";
});

// ---- t() and keys -----------------------------------------------------------------------------------

describe("t()", () => {
  it("is English identity with {name} placeholders; a table translates, missing entries fall back", () => {
    expect(t("Take {layer}", { layer: "Lower third" })).toBe("Take Lower third");
    expect(t("Go to {page}", { other: 1 })).toBe("Go to {page}");
    expect(t("{n} items", { n: 3 })).toBe("3 items");
    setMessages({ "Take {layer}": "Ta {layer}" });
    expect(t("Take {layer}", { layer: "Navn" })).toBe("Ta Navn");
    expect(t("Clear {layer}", { layer: "Navn" })).toBe("Clear Navn");
  });
});

describe("keys", () => {
  it("one canonical spelling", () => {
    expect(normalizeKeys("ctrl+shift+p")).toBe("Ctrl+Shift+P");
    expect(normalizeKeys("Shift+Ctrl+P")).toBe("Ctrl+Shift+P");
    expect(normalizeKeys("Shift+Esc")).toBe("Shift+Escape");
    expect(normalizeKeys("shift+f3")).toBe("Shift+F3");
    expect(normalizeKeys("Cmd+K")).toBe("Ctrl+K");
    expect(normalizeKeys("Alt+Shift+h")).toBe("Alt+Shift+H");
    expect(normalizeKeys("Ctrl+;")).toBe("Ctrl+;");
    for (const bad of ["", "Ctrl+", "Ctrl+Ctrl+K", "Hyper+K", "F25", "Ctrl+ä", "KK"]) expect(normalizeKeys(bad)).toBeNull();
  });

  it("an event's keys: letters by physical key, F-keys and Escape by name, punctuation also without Shift", () => {
    expect(eventKeys(key({ key: "p", code: "KeyP", ctrlKey: true, shiftKey: true }))).toEqual(["Ctrl+Shift+P"]);
    expect(eventKeys(key({ key: "F3", shiftKey: true }))).toEqual(["Shift+F3"]);
    expect(eventKeys(key({ key: "Escape", shiftKey: true }))).toEqual(["Shift+Escape"]);
    expect(eventKeys(key({ key: "å", code: "KeyA", altKey: true }))).toEqual(["Alt+A"]);
    expect(eventKeys(key({ key: ";", code: "Comma", ctrlKey: true, shiftKey: true }))).toEqual(["Ctrl+Shift+;", "Ctrl+;"]);
    expect(eventKeys(key({ key: "Shift", shiftKey: true }))).toEqual([]);
  });
});

// ---- Registry ---------------------------------------------------------------------------------------

describe("registry", () => {
  it("register / unregister; the same id replaces; an old unregister never drops the newer one", () => {
    const a1 = cmd("a");
    const a2 = cmd("a", { title: "A2" });
    const off1 = keep(registerCommands([a1, cmd("b")]));
    expect(getCommand("a")).toBe(a1);
    const off2 = keep(registerCommands([a2]));
    expect(getCommand("a")).toBe(a2);
    expect(commands().filter((c) => c.id === "a")).toHaveLength(1);
    off1();
    expect(getCommand("a")).toBe(a2);
    expect(getCommand("b")).toBeUndefined();
    off2();
    expect(getCommand("a")).toBeUndefined();
  });

  it("context: the shell's route and project, pages' patches on top, dropped again", () => {
    expect(routeOf("/templates/abc")).toBe("templateEditor");
    expect(routeOf("/screens")).toBe("screens");
    expect(routeOf("/flows/x")).toBe("flowEditor");
    expect(routeOf("/")).toBe("picker");
    const drop1 = keep(provideContext({ route: "control", project: true }));
    const drop2 = keep(provideContext({ screenId: "s1", studio: true }));
    expect(currentContext()).toMatchObject({ route: "control", project: true, screenId: "s1", studio: true });
    drop2();
    expect(currentContext().screenId).toBeNull();
    drop1();
    expect(currentContext().route).toBe("dashboard");
  });

  it("runCommand: only when allowed and not disabled; errors toast; providers' items run too", async () => {
    const hidden = cmd("hidden", { when: (c) => c.route === "control" });
    const off = cmd("off", { disabled: () => "Nope" });
    const boom = cmd("boom", { run: () => Promise.reject(new Error("disk full")) });
    keep(registerCommands([hidden, off, boom]));
    expect(await runCommand("hidden")).toBe(false);
    expect(await runCommand("off")).toBe(false);
    expect(await runCommand("nope")).toBe(false);
    keep(provideContext({ route: "control" }));
    expect(await runCommand("hidden")).toBe(true);
    expect(hidden.run).toHaveBeenCalledWith(expect.objectContaining({ route: "control" }));
    expect(off.run).not.toHaveBeenCalled();
    expect(await runCommand("boom")).toBe(true);
    expect(toasts.list.at(-1)).toMatchObject({ kind: "error", message: "disk full" });

    const item = cmd("item:s1", { section: "items", scope: "screen" });
    const items = vi.fn(() => [item]);
    keep(registerProvider({ id: "screens", items }));
    expect(await runCommand("item:s1")).toBe(true);
    expect(item.run).toHaveBeenCalled();
  });

  it("Recent: newest first, 5 at most, per project (ui-state), noRecent and unknown ids skipped", async () => {
    const ids = ["c1", "c2", "c3", "c4", "c5", "c6"];
    keep(registerCommands([...ids.map((id) => cmd(id)), cmd("quiet", { noRecent: true })]));
    for (const id of ids) await runCommand(id);
    await runCommand("c3");
    await runCommand("quiet");
    expect(recentIds()).toEqual(["c3", "c6", "c5", "c4", "c2"]);
    expect((api.setUiState.mock.calls.at(-1)![0] as UiState).palette?.recent).toEqual(["c3", "c6", "c5", "c4", "c2"]);
    // Unresolvable ids are skipped when listing; another project has its own list.
    uiState.set(ui({}, ["gone", "c1"]));
    expect(recentIds()).toEqual(["c1"]);
    uiState.set(null);
    expect(recentIds()).toEqual([]);
    expect(await runCommand("c1")).toBe(true);
  });

  it("follow-up: pick and text steps run with the value; validation blocks; keys open the step", async () => {
    const picked = vi.fn();
    const named = vi.fn();
    keep(
      registerCommands([
        cmd("variant", {
          title: "Set variant…",
          run: undefined,
          defaultKeys: ["Ctrl+Alt+V"],
          next: { kind: "pick", placeholder: "Choose a variant", options: () => [{ id: "split", title: "Split" }], run: picked },
        }),
        cmd("rename", {
          title: "Rename…",
          run: undefined,
          next: { kind: "text", placeholder: "Name", validate: (s) => (s.trim() ? null : "Enter a name"), run: named },
        }),
      ]),
    );
    expect(await runCommand("variant")).toBe(false);
    expect(await runFollowUp("variant", "split")).toBe(true);
    expect(picked).toHaveBeenCalledWith("split", expect.anything());
    expect(await runFollowUp("rename", "  ")).toBe(false);
    expect(await runFollowUp("rename", "Show")).toBe(true);
    expect(named).toHaveBeenCalledWith("Show", expect.anything());
    expect(recentIds()).toEqual(["rename", "variant"]);

    const open = vi.fn();
    setFollowUpOpener(open);
    keep(() => setFollowUpOpener(null));
    expect(runShortcut(key({ key: "v", code: "KeyV", ctrlKey: true, altKey: true }))).toBe(true);
    expect(open).toHaveBeenCalledWith("variant");
  });
});

// ---- Shortcuts --------------------------------------------------------------------------------------

describe("shortcuts", () => {
  it("bindings override defaults: machine table, [] unbinds, null restores", async () => {
    const run = vi.fn();
    keep(registerCommands([cmd("shell.pin", { defaultKeys: ["Ctrl+B"], run })]));
    expect(bindingOf("shell.pin")).toEqual({ commandId: "shell.pin", keys: ["Ctrl+B"], source: "default" });
    expect(runShortcut(key({ key: "b", code: "KeyB", ctrlKey: true }))).toBe(true);
    expect(run).toHaveBeenCalledTimes(1);

    await setKeys("shell.pin", ["ctrl+alt+p"]);
    expect((api.setPrefs.mock.calls.at(-1)![0] as Prefs).keybindings).toEqual({ "shell.pin": ["Ctrl+Alt+P"] });
    expect(bindingOf("shell.pin").source).toBe("machine");
    expect(runShortcut(key({ key: "b", code: "KeyB", ctrlKey: true }))).toBe(false);
    expect(runShortcut(key({ key: "p", code: "KeyP", ctrlKey: true, altKey: true }))).toBe(true);
    expect(run).toHaveBeenCalledTimes(2);

    await setKeys("shell.pin", []);
    expect(keysFor("shell.pin")).toEqual([]);
    await setKeys("shell.pin", null);
    expect(keysFor("shell.pin")).toEqual(["Ctrl+B"]);
    expect(prefs.value?.keybindings).toEqual({});
    // The defaults again are stored as "default"; a non-key is refused.
    await setKeys("shell.pin", ["Ctrl+B"]);
    expect(prefs.value?.keybindings).toEqual({});
    await expect(setKeys("shell.pin", ["Hyper+X"])).rejects.toThrow("can't be used");
  });

  it("layer and cue keys come from the project table (control.hotkeys); setKeys writes it", async () => {
    uiState.set(ui({ hotkeys: { "s1/lower": "F5", "macro/m1": "Shift+F6" } }));
    keep(provideContext({ route: "control" }));
    const toggle = vi.fn();
    const go = vi.fn();
    keep(
      registerCommands([
        cmd("live.toggle:s1/lower", { section: "live", live: true, keyScope: "control", run: toggle }),
        cmd("cue.go:m1", { section: "live", live: true, keyScope: "control", run: go }),
      ]),
    );
    expect(bindingOf("live.toggle:s1/lower")).toEqual({ commandId: "live.toggle:s1/lower", keys: ["F5"], source: "project" });
    expect(keysFor("cue.go:m1")).toEqual(["Shift+F6"]);
    runShortcut(key({ key: "F5" }));
    runShortcut(key({ key: "F6", shiftKey: true }));
    expect(toggle).toHaveBeenCalledTimes(1);
    expect(go).toHaveBeenCalledTimes(1);

    // Assigning a taken key moves it (one target per key); a cue key replaces the legacy spelling.
    await setKeys("cue.go:m1", ["F5"]);
    expect(uiState.value?.control.hotkeys).toEqual({ "flow/m1": "F5" });
    await setKeys("live.toggle:s1/lower", ["shift+f2"]);
    expect(uiState.value?.control.hotkeys).toEqual({ "flow/m1": "F5", "s1/lower": "Shift+F2" });
    await setKeys("cue.go:m1", null);
    expect(uiState.value?.control.hotkeys).toEqual({ "s1/lower": "Shift+F2" });
    await expect(setKeys("live.toggle:s1/lower", ["Ctrl+G"])).rejects.toThrow("F1 to F12");
    expect(prefs.value?.keybindings).toEqual({});
  });

  it("key scopes: a command's keys work only on its route", () => {
    const run = vi.fn();
    keep(registerCommands([cmd("editor.grid", { defaultKeys: ["Ctrl+G"], keyScope: "screenEditor", run })]));
    keep(provideContext({ route: "templates" }));
    expect(runShortcut(key({ key: "g", code: "KeyG", ctrlKey: true }))).toBe(false);
    keep(provideContext({ route: "screenEditor" }));
    expect(runShortcut(key({ key: "g", code: "KeyG", ctrlKey: true }))).toBe(true);
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("typing: plain keys and live commands never fire; Ctrl keys do unless notWhileTyping; whileTyping always", () => {
    const input = document.createElement("input");
    const plain = cmd("plain", { defaultKeys: ["G"] });
    const ctrl = cmd("ctrl", { defaultKeys: ["Ctrl+G"] });
    const guarded = cmd("guarded", { defaultKeys: ["Ctrl+H"], notWhileTyping: true });
    const liveCmd = cmd("live", { defaultKeys: ["Ctrl+L"], live: true });
    const panic = cmd("panic", { defaultKeys: ["Shift+Escape"], live: true, whileTyping: true });
    keep(registerCommands([plain, ctrl, guarded, liveCmd, panic]));
    expect(runShortcut(key({ key: "g", code: "KeyG", target: input }))).toBe(false);
    expect(runShortcut(key({ key: "g", code: "KeyG", ctrlKey: true, target: input }))).toBe(true);
    expect(runShortcut(key({ key: "h", code: "KeyH", ctrlKey: true, target: input }))).toBe(false);
    expect(runShortcut(key({ key: "l", code: "KeyL", ctrlKey: true, target: input }))).toBe(false);
    expect(runShortcut(key({ key: "Escape", shiftKey: true, target: input }))).toBe(true);
    expect([plain, ctrl, guarded, liveCmd, panic].map((c) => vi.mocked(c.run!).mock.calls.length)).toEqual([0, 1, 0, 0, 1]);
  });

  it("repeats are swallowed unless repeatable; never for live commands", () => {
    const liveCmd = cmd("take", { defaultKeys: ["F1"], live: true });
    const nudge = cmd("nudge", { defaultKeys: ["Alt+ArrowLeft"], repeatable: true });
    keep(registerCommands([liveCmd, nudge]));
    expect(runShortcut(key({ key: "F1", repeat: true }))).toBe(true);
    expect(liveCmd.run).not.toHaveBeenCalled();
    expect(runShortcut(key({ key: "ArrowLeft", altKey: true, repeat: true }))).toBe(true);
    expect(nudge.run).toHaveBeenCalledTimes(1);
  });

  it("a conflict runs nothing and toasts, with a link to Keyboard shortcuts", async () => {
    const a = cmd("a", { title: "Toggle grid", defaultKeys: ["Ctrl+G"] });
    const b = cmd("b", { title: "Go to {page}", params: () => ({ page: "Flows" }), keyScope: "screenEditor" });
    keep(registerCommands([a, b]));
    await setKeys("b", ["Ctrl+G"]);
    keep(provideContext({ route: "screenEditor" }));
    expect(runShortcut(key({ key: "g", code: "KeyG", ctrlKey: true }))).toBe(true);
    expect(a.run).not.toHaveBeenCalled();
    expect(b.run).not.toHaveBeenCalled();
    const toast = toasts.list.at(-1)!;
    expect(toast.message).toBe("Ctrl+G is assigned to Toggle grid, Go to Flows. Nothing ran.");
    toast.action!.run();
    expect(api.openPreferences).toHaveBeenCalledWith("shortcuts");
    // Elsewhere only one of them applies: no conflict at run time.
    keep(provideContext({ route: "dashboard" }));
    expect(runShortcut(key({ key: "g", code: "KeyG", ctrlKey: true }))).toBe(true);
    expect(a.run).toHaveBeenCalledTimes(1);
  });

  it("the shell's capture-phase listener wins inside a CodeMirror-like contenteditable", () => {
    const editor = document.createElement("div");
    editor.contentEditable = "true";
    document.body.append(editor);
    const editorKeys = vi.fn();
    editor.addEventListener("keydown", editorKeys);
    const open = cmd("palette.open", { defaultKeys: ["Ctrl+K"], whileTyping: true, noRecent: true });
    keep(registerCommands([open]));
    keep(installShortcuts({ route: "templateEditor" }));
    const e = key({ key: "k", code: "KeyK", ctrlKey: true });
    editor.dispatchEvent(e);
    expect(open.run).toHaveBeenCalledTimes(1);
    expect(e.defaultPrevented).toBe(true);
    expect(editorKeys).not.toHaveBeenCalled();
    // Typing reaches the editor.
    editor.dispatchEvent(key({ key: "k", code: "KeyK" }));
    expect(editorKeys).toHaveBeenCalledTimes(1);
  });
});

// ---- Conflicts --------------------------------------------------------------------------------------

describe("conflicts", () => {
  it("same keys and overlapping key scopes, including project keys of commands not registered now", async () => {
    keep(
      registerCommands([
        cmd("nav.flows", { defaultKeys: ["Ctrl+5"] }),
        cmd("editor.grid", { defaultKeys: ["Ctrl+G"], keyScope: "screenEditor" }),
        cmd("flow.grid", { defaultKeys: ["Ctrl+G"], keyScope: "flowEditor" }),
        cmd("other", { defaultKeys: ["F5"], keyScope: "dashboard" }),
        cmd("global.f6", { defaultKeys: ["F6"] }),
      ]),
    );
    // Different routes: no conflict.
    expect(conflicts()).toEqual([]);
    await setKeys("nav.flows", ["Ctrl+G"]);
    expect(conflicts()).toEqual([{ keys: "Ctrl+G", commandIds: ["nav.flows", "editor.grid", "flow.grid"] }]);
    await setKeys("nav.flows", null);

    // Control isn't open: its layer and cue keys still count (key scope "control").
    uiState.set(ui({ hotkeys: { "s1/lower": "F5", "flow/f1": "F6", "macro/f1": "F7" } }));
    expect(projectBindings().map((b) => [b.commandId, b.keys[0]])).toEqual([
      ["live.toggle:s1/lower", "F5"],
      ["cue.go:f1", "F6"],
    ]);
    expect(conflicts()).toEqual([{ keys: "F6", commandIds: ["global.f6", "cue.go:f1"] }]);
  });
});

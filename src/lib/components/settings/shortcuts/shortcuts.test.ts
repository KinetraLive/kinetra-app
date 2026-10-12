// Preferences > Keyboard shortcuts: the pure model (groups, filter, recording) and the list
// (record, remove, reset, conflicts, layer and cue keys by name, the inert page commands).
import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { FlowInfo, LiveScreenView, Prefs, UiState } from "$lib/types";

const api = vi.hoisted(() => ({
  getPrefs: vi.fn(),
  setPrefs: vi.fn(async (p: unknown) => p),
  getUiState: vi.fn(),
  setUiState: vi.fn(async (u: unknown) => u),
  listScreens: vi.fn(),
  listFlows: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  on: () => () => {},
  toAppError: (e: unknown) => ({ code: "internal", message: e instanceof Error ? e.message : String(e) }),
}));
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/flows", () => api);
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));

import { bindingOf } from "$lib/commands/bindings.svelte";
import { commands, registerCommands, runShortcut, type Command } from "$lib/commands/registry.svelte";
import { prefs } from "$lib/stores/prefs.svelte";
import { uiState } from "$lib/stores/uiState.svelte";
import { conflictsById, groupRows, keysAfterRecording, keysAfterRemoving, recordedKey, type ShortcutRow } from "./model";
import Shortcuts from "./Shortcuts.svelte";

const basePrefs = (keybindings: Record<string, string[]> = {}): Prefs => ({
  theme: "dark",
  backup: { enabled: true, intervalMinutes: 15, retention: 10 },
  keybindings,
});
const ui = (hotkeys: Record<string, string> = {}): UiState => ({
  sidebarPinned: false,
  control: { view: "grid", gridColumns: 2, selectedScreenId: null, hiddenScreenIds: [], liveEdit: "onTake", hotkeys, automationOff: false },
});
const row = (id: string, keys: string[], patch: Partial<ShortcutRow> = {}): ShortcutRow => ({
  id,
  title: id,
  section: "settings",
  scope: "global",
  binding: { commandId: id, keys, source: "default" },
  project: false,
  ...patch,
});
const key = (k: string, init: Partial<KeyboardEventInit> = {}) => ({ key: k, code: "", shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, ...init });

describe("model", () => {
  it("groups by palette section in order and filters by title, keys and where", () => {
    const rows = [
      row("a", ["Ctrl+G"], { title: "Toggle grid", section: "edit", scope: "screenEditor" }),
      row("b", ["Ctrl+2"], { title: "Go to Templates", section: "goto" }),
      row("c", [], { title: "Panic", section: "live", scope: "control" }),
    ];
    expect(groupRows(rows, "").map((g) => g.section)).toEqual(["live", "goto", "edit"]);
    expect(groupRows(rows, "grid")[0].rows.map((r) => r.id)).toEqual(["a"]);
    expect(groupRows(rows, "ctrl+2")[0].rows.map((r) => r.id)).toEqual(["b"]);
    expect(groupRows(rows, "layout editor")[0].rows.map((r) => r.id)).toEqual(["a"]);
    expect(groupRows(rows, "zzz")).toEqual([]);
  });

  it("recording: Esc alone cancels, a lone modifier waits, anything else is the binding", () => {
    expect(recordedKey(key("Escape"))).toEqual({ kind: "cancel" });
    expect(recordedKey(key("Escape", { shiftKey: true }))).toEqual({ kind: "keys", keys: "Shift+Escape" });
    expect(recordedKey(key("Control", { ctrlKey: true }))).toBeNull();
    expect(recordedKey(key("g", { code: "KeyG", ctrlKey: true }))).toEqual({ kind: "keys", keys: "Ctrl+G" });
  });

  it("a command gains the recorded key (4 at most); a layer / cue key is replaced; removing", () => {
    const r = row("a", ["Ctrl+K", "Ctrl+P"]);
    expect(keysAfterRecording(r, "Ctrl+J")).toEqual(["Ctrl+K", "Ctrl+P", "Ctrl+J"]);
    expect(keysAfterRecording(r, "Ctrl+K")).toEqual(["Ctrl+P", "Ctrl+K"]);
    expect(keysAfterRecording(row("a", ["A", "B", "C", "D"]), "E")).toEqual(["B", "C", "D", "E"]);
    expect(keysAfterRecording(row("live.toggle:s/l", ["F1"], { project: true }), "F2")).toEqual(["F2"]);
    // Fixing a conflict replaces the conflicting key.
    expect(keysAfterRecording(r, "Ctrl+J", "Ctrl+K")).toEqual(["Ctrl+P", "Ctrl+J"]);
    expect(keysAfterRemoving(r, "Ctrl+K")).toEqual(["Ctrl+P"]);
    expect(keysAfterRemoving(row("x", ["F1"], { project: true }), "F1")).toBeNull();
  });

  it("conflicts by command id", () => {
    const m = conflictsById([{ keys: "Ctrl+G", commandIds: ["a", "b"] }]);
    expect(m.get("a")?.[0].keys).toBe("Ctrl+G");
    expect(m.has("c")).toBe(false);
  });
});

// ---- The list ---------------------------------------------------------------------------------------

const SCREENS: LiveScreenView[] = [
  {
    screen: { id: "s1", name: "Main", width: 1920, height: 1080, background: "transparent" },
    layers: [{ id: "l1", name: "Lower third", onAir: false, fields: {}, fieldDefs: [], pending: false }],
  },
];
const FLOWS = [
  { id: "f1", name: "Opening", armed: true, nodes: [{ id: "n", type: "trigger.cue", position: { x: 0, y: 0 }, config: { label: null } }], edges: [] },
] as unknown as FlowInfo[];

let offs: (() => void)[] = [];
const cmd = (id: string, title: string, patch: Partial<Command> = {}): Command => ({ id, title, section: "settings", run: vi.fn(), ...patch });

beforeEach(() => {
  vi.clearAllMocks();
  api.listScreens.mockResolvedValue(SCREENS);
  api.listFlows.mockResolvedValue(FLOWS);
  prefs.set(basePrefs());
  uiState.set(ui({ "s1/l1": "F1" }));
  offs.push(
    registerCommands([
      cmd("shell.theme", "Toggle light / dark theme"),
      cmd("nav.templates", "Go to Templates", { section: "goto", defaultKeys: ["Ctrl+2"] }),
    ]),
  );
});
afterEach(() => {
  cleanup();
  for (const off of offs.reverse()) off();
  offs = [];
});

const rowOf = (title: string) => {
  const el = screen.getByText(title, { selector: ".title" }).closest("li");
  if (!el) throw new Error(title);
  return el as HTMLElement;
};

async function mount() {
  render(Shortcuts);
  await vi.waitFor(() => expect(screen.getByText("Take or clear Lower third (Main)")).toBeTruthy());
  await vi.waitFor(() => expect(screen.getByText("GO Opening")).toBeTruthy());
}

describe("Keyboard shortcuts list", () => {
  it("lists commands by section with their keys in mono; layer and cue keys by name; pages' commands too", async () => {
    await mount();
    const labels = screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent);
    expect(labels).toEqual(["Live", "Go to", "Create", "Edit", "Settings"]);
    // The shell's commands are listed in the Preferences window too (no shell there).
    expect(within(rowOf("Preferences…")).getByText("Ctrl+,")).toBeTruthy();
    expect(within(rowOf("Show all commands")).getByText("Ctrl+K")).toBeTruthy();
    const tpl = rowOf("Go to Templates");
    expect(within(tpl).getByText("Ctrl+2").classList.contains("mono")).toBe(true);
    expect(within(rowOf("Take or clear Lower third (Main)")).getByText("F1")).toBeTruthy();
    expect(within(rowOf("Take or clear Lower third (Main)")).getByText("In Control")).toBeTruthy();
    // Layout editor, flow editor and Control commands are listed while those pages are closed.
    expect(within(rowOf("Toggle grid")).getByText("Ctrl+G")).toBeTruthy();
    expect(within(rowOf("Toggle grid")).getByText("In the layout editor")).toBeTruthy();
    expect(within(rowOf("Deploy flow")).getByText("Ctrl+Enter")).toBeTruthy();
    expect(within(rowOf("Panic: cut everything off air")).getByText("Shift+Escape")).toBeTruthy();
    // …as inert copies: never run, never listed in the palette.
    const grid = commands().find((c) => c.id === "editor.grid");
    expect(grid?.when?.({ route: "screenEditor", project: true, screenId: null, layerIds: [], studio: false })).toBe(false);
  });

  it("search filters the rows", async () => {
    await mount();
    await fireEvent.input(screen.getByRole("searchbox", { name: "Search shortcuts" }), { target: { value: "theme" } });
    expect(screen.getAllByRole("listitem").map((li) => li.querySelector(".title")?.textContent)).toEqual(["Toggle light / dark theme"]);
  });

  it("records a key (no command runs meanwhile; Esc cancels), removes it, resets to default", async () => {
    await mount();
    await fireEvent.click(within(rowOf("Go to Templates")).getByRole("button", { name: "Add key for Go to Templates" }));
    expect(within(rowOf("Go to Templates")).getByRole("status").textContent).toContain("Esc cancels");
    // Paused: the shell resolves nothing while recording.
    const probe = new KeyboardEvent("keydown", { key: "2", code: "Digit2", ctrlKey: true });
    expect(runShortcut(probe)).toBe(false);
    await fireEvent.keyDown(window, { key: "Escape" });
    await tick();
    expect(within(rowOf("Go to Templates")).queryByRole("status")).toBeNull();
    expect(api.setPrefs).not.toHaveBeenCalled();

    await fireEvent.click(within(rowOf("Go to Templates")).getByRole("button", { name: "Add key for Go to Templates" }));
    await fireEvent.keyDown(window, { key: "t", code: "KeyT", ctrlKey: true, altKey: true });
    await vi.waitFor(() => expect(bindingOf("nav.templates").keys).toEqual(["Ctrl+2", "Ctrl+Alt+T"]));
    expect(prefs.value?.keybindings).toEqual({ "nav.templates": ["Ctrl+2", "Ctrl+Alt+T"] });
    await tick();
    expect(within(rowOf("Go to Templates")).getByText("Ctrl+Alt+T")).toBeTruthy();

    await fireEvent.click(within(rowOf("Go to Templates")).getByRole("button", { name: "Remove Ctrl+2 from Go to Templates" }));
    await vi.waitFor(() => expect(bindingOf("nav.templates").keys).toEqual(["Ctrl+Alt+T"]));

    await fireEvent.click(within(rowOf("Go to Templates")).getByRole("button", { name: "Reset" }));
    await vi.waitFor(() => expect(bindingOf("nav.templates")).toEqual({ commandId: "nav.templates", keys: ["Ctrl+2"], source: "default" }));
  });

  it("a layer key is F1–F12 in the project table; anything else shows why", async () => {
    await mount();
    const r = () => rowOf("Take or clear Lower third (Main)");
    await fireEvent.click(within(r()).getByRole("button", { name: "Change for Take or clear Lower third (Main)" }));
    expect(within(r()).getByRole("status").textContent).toContain("F1–F12");
    await fireEvent.keyDown(window, { key: "F5", shiftKey: true });
    await vi.waitFor(() => expect(uiState.value?.control.hotkeys).toEqual({ "s1/l1": "Shift+F5" }));
    await tick();
    await fireEvent.click(within(r()).getByRole("button", { name: "Change for Take or clear Lower third (Main)" }));
    await fireEvent.keyDown(window, { key: "g", code: "KeyG", ctrlKey: true });
    await vi.waitFor(() => expect(within(r()).getByRole("alert").textContent).toContain("F1 to F12"));
    await fireEvent.click(within(r()).getByRole("button", { name: "Remove Shift+F5 from Take or clear Lower third (Main)" }));
    await vi.waitFor(() => expect(uiState.value?.control.hotkeys).toEqual({}));
  });

  it("shows conflicts explicitly (both commands, a fix for each), never resolving them", async () => {
    prefs.set(basePrefs({ "shell.theme": ["F1"] }));
    await mount();
    const panel = screen.getByRole("region", { name: "1 conflict" });
    expect(panel.textContent).toContain("F1");
    expect(panel.textContent).toContain("Toggle light / dark theme");
    expect(panel.textContent).toContain("Take or clear Lower third (Main)");
    expect(within(rowOf("Toggle light / dark theme")).getByText("Conflict")).toBeTruthy();
    expect(within(rowOf("Take or clear Lower third (Main)")).getByText("Conflict")).toBeTruthy();
    // Both keep their keys.
    expect(bindingOf("shell.theme").keys).toEqual(["F1"]);
    expect(uiState.value?.control.hotkeys).toEqual({ "s1/l1": "F1" });
    // The fix: change one of them.
    await fireEvent.click(within(panel).getByRole("button", { name: "Change Toggle light / dark theme" }));
    await fireEvent.keyDown(window, { key: "F2" });
    await vi.waitFor(() => expect(screen.queryByRole("region", { name: /conflict/ })).toBeNull());
  });
});

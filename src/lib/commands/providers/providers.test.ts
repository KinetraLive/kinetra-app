// Palette providers: the project items (screens, templates, sources, flows and cues, slides,
// schedule items, Preferences sections), Control's live items and the layout editor's layers, plus
// the follow-ups ("Set variant…", "Rename screen…") and the Edit section.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Command, CommandContext } from "../registry.svelte";
import type { FlowInfo, LiveLayerView, LiveScreenView, ScheduleView, SourceSummary, TemplateSummary, UiState } from "$lib/types";

const api = vi.hoisted(() => ({
  openPreferences: vi.fn(async (_section?: string) => {}),
  listScreens: vi.fn(),
  listTemplates: vi.fn(),
  listSources: vi.fn(),
  listFlows: vi.fn(),
  scheduleView: vi.fn(),
  runFlow: vi.fn(async () => {}),
  stepSource: vi.fn(),
  scheduleAction: vi.fn(),
  getUiState: vi.fn(),
  setUiState: vi.fn(async (u: unknown) => u),
  getPrefs: vi.fn(),
  setPrefs: vi.fn(async (p: unknown) => p),
}));
vi.mock("$lib/api", () => ({
  ...api,
  on: () => () => {},
  toAppError: (e: unknown) => ({ code: "internal", message: String(e) }),
}));
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/templates", () => api);
vi.mock("$lib/api/sources", () => api);
vi.mock("$lib/api/flows", () => api);
vi.mock("$lib/api/recipes", () => api);
vi.mock("$lib/api/screens", () => api);
const nav = vi.hoisted(() => ({ goto: vi.fn(async (_url: string) => {}) }));
vi.mock("$app/navigation", () => nav);
const update = vi.hoisted(() => ({ updateLook: vi.fn(async () => null) }));
vi.mock("$lib/components/templates/update", () => update);

import { prefs } from "$lib/stores/prefs.svelte";
import { uiState } from "$lib/stores/uiState.svelte";
import { controlCommands, controlProvider } from "$lib/components/control/commands.svelte";
import type { ControlCtl } from "$lib/components/control/control.svelte";
import { editorCommands, editorProvider, renameCommand } from "$lib/components/screens/editor/commands";
import { flowEditorCommands } from "$lib/components/flows/graph/commands";
import { variantCommands } from "$lib/components/screens/variants.svelte";
import { prepare, search, sectionOrder } from "../palette/search";
import { providerItems, registerCommands, titleOf } from "../registry.svelte";
import { watchItems } from "./project.svelte";

const ctx = (patch: Partial<CommandContext> = {}): CommandContext => ({
  route: "dashboard",
  project: true,
  screenId: null,
  layerIds: [],
  studio: false,
  ...patch,
});

const layer = (id: string, name: string, patch: Partial<LiveLayerView> = {}): LiveLayerView => ({
  id,
  name,
  onAir: false,
  fields: {},
  fieldDefs: [],
  pending: false,
  ...patch,
});
const view = (id: string, name: string, layers: LiveLayerView[] = []): LiveScreenView => ({
  screen: { id, name, width: 1920, height: 1080, background: "transparent" },
  layers,
});

const SCREENS = [view("s1", "Main", [layer("l1", "Lower third")]), view("__preview", "Preview"), view("test", "Test")];
const TEMPLATES: TemplateSummary[] = [
  { id: "t1", name: "Lower third", description: "Name and title", preset: null, fieldCount: 2, variableCount: 0, variantCount: 1, updatedAt: "", presetUpdate: null },
];
const source = (id: string, name: string, patch: Partial<SourceSummary> = {}): SourceSummary => ({
  id,
  name,
  kind: "text",
  rowCount: 3,
  index: 0,
  current: {},
  columns: [],
  usedBy: [],
  updatedAt: "",
  auto: null,
  nextAt: null,
  nowId: null,
  nextId: null,
  ...patch,
});
const SOURCES = [
  source("src1", "Guests"),
  source("slides", "Sponsors", { auto: { everyMs: 5000, loop: true, transition: "update" } }),
  source("sch", "Run of show", { kind: "schedule" }),
];
const flow = (id: string, name: string, cue: boolean, armed = true): FlowInfo =>
  ({
    id,
    name,
    armed,
    nodes: cue ? [{ id: "n1", type: "trigger.cue", position: { x: 0, y: 0 }, config: { label: `${name} cue` } }] : [],
    edges: [],
    warnings: [],
    running: false,
    readOnly: false,
  }) as unknown as FlowInfo;
const FLOWS = [flow("f1", "Opening", true), flow("f2", "Rotate", false), flow("f3", "Off", true, false)];
const SCHEDULE: ScheduleView = {
  sourceId: "sch",
  nowId: "i1",
  nextId: "i2",
  computedAt: 1,
  canUndo: false,
  items: [
    { id: "i0", title: "Doors", location: "", status: "previous", start: "", end: "", startMs: 0, endMs: 0, moved: false },
    { id: "i1", title: "Keynote", location: "Hall A", status: "now", start: "", end: "", startMs: 0, endMs: 0, moved: false },
    { id: "i2", title: "Panel", location: "", status: "next", start: "", end: "", startMs: 0, endMs: 0, moved: false },
    { id: "i3", title: "Lunch", location: "", status: "upcoming", start: "", end: "", startMs: 0, endMs: 0, moved: false },
  ],
};

const UI: UiState = {
  sidebarPinned: false,
  control: {
    view: "grid",
    gridColumns: 2,
    selectedScreenId: null,
    hiddenScreenIds: [],
    liveEdit: "onTake",
    hotkeys: {},
    automationOff: false,
  },
};

const ids = (cmds: readonly Command[]) => cmds.map((c) => c.id);
const byId = (cmds: readonly Command[], id: string) => {
  const c = cmds.find((x) => x.id === id);
  if (!c) throw new Error(`no ${id} in ${ids(cmds).join(", ")}`);
  return c;
};

let cleanups: (() => void)[] = [];

beforeEach(() => {
  vi.clearAllMocks();
  api.listScreens.mockResolvedValue(SCREENS);
  api.listTemplates.mockResolvedValue(TEMPLATES);
  api.listSources.mockResolvedValue(SOURCES);
  api.listFlows.mockResolvedValue(FLOWS);
  api.scheduleView.mockResolvedValue(SCHEDULE);
  api.stepSource.mockImplementation(async (id: string) => SOURCES.find((s) => s.id === id));
  api.scheduleAction.mockResolvedValue(SCHEDULE);
  uiState.set(UI);
});
afterEach(() => {
  for (const off of cleanups.reverse()) off();
  cleanups = [];
});

async function loaded() {
  cleanups.push(watchItems());
  await vi.waitFor(() => expect(providerItems(ctx()).some((c) => c.id.startsWith("schedule."))).toBe(true));
}

describe("project items", () => {
  it("lists screens (@), templates (#), sources, flows and Preferences (/) with stable ids", async () => {
    await loaded();
    const items = providerItems(ctx());
    // Reserved screens are never listed.
    expect(ids(items).filter((i) => i.startsWith("screen."))).toEqual(["screen.edit:s1", "screen.control:s1"]);
    expect(titleOf(byId(items, "screen.edit:s1"))).toBe("Go to Main");
    expect(titleOf(byId(items, "screen.control:s1"))).toBe("Open Main in Control");
    expect(byId(items, "screen.edit:s1").scope).toBe("screen");
    expect(titleOf(byId(items, "template.edit:t1"))).toBe("Edit Lower third");
    expect(byId(items, "template.edit:t1").scope).toBe("template");
    expect(titleOf(byId(items, "source.open:src1"))).toBe("Open Guests");
    expect(titleOf(byId(items, "flow.open:f2"))).toBe("Open Rotate");
    const prefs = items.filter((c) => c.id.startsWith("settings.section:"));
    expect(prefs.map(titleOf)).toContain("Preferences: Server");
    expect(prefs.map(titleOf)).toContain("Keyboard shortcuts");
    expect(prefs.every((c) => c.scope === "setting" && c.section === "settings")).toBe(true);
    // Without a project: nothing.
    expect(providerItems(ctx({ project: false }))).toEqual([]);
  });

  it("Simple mode: no Open {flow} (the flow editor is hidden); cues still listed", async () => {
    await loaded();
    prefs.set({ theme: "dark", backup: { enabled: true, intervalMinutes: 15, retention: 10 }, uiMode: "simple" });
    try {
      const items = providerItems(ctx());
      expect(ids(items).filter((i) => i.startsWith("flow.open:"))).toEqual([]);
      expect(ids(items)).toContain("cue.go:f1");
    } finally {
      prefs.set(null);
    }
  });

  it("Update {template} to the latest look for outdated preset templates (Edit), not in the template editor", async () => {
    const outdated: TemplateSummary = {
      ...TEMPLATES[0],
      id: "t2",
      name: "Strap",
      preset: { id: "lower-third", version: 1 },
      presetUpdate: { from: 1, to: 2 },
    };
    api.listTemplates.mockResolvedValue([...TEMPLATES, outdated]);
    await loaded();
    await vi.waitFor(() => expect(ids(providerItems(ctx()))).toContain("template.update:t2"));
    const items = providerItems(ctx());
    expect(ids(items).filter((i) => i.startsWith("template.update:"))).toEqual(["template.update:t2"]);
    const c = byId(items, "template.update:t2");
    expect(titleOf(c)).toBe("Update Strap to the latest look");
    expect(c.section).toBe("edit");
    expect(c.scope).toBe("template");
    await c.run?.(ctx());
    expect(update.updateLook).toHaveBeenCalledWith(outdated, expect.objectContaining({ after: expect.any(Function) }));
    // The template editor registers its own (it saves its working copy first).
    expect(ids(providerItems(ctx({ route: "templateEditor" }))).some((i) => i.startsWith("template.update:"))).toBe(false);
  });

  it("navigation items go where they say; Open in Control selects the screen first", async () => {
    await loaded();
    const items = providerItems(ctx());
    await byId(items, "screen.edit:s1").run?.(ctx());
    expect(nav.goto).toHaveBeenLastCalledWith("/screens/s1");
    await byId(items, "template.edit:t1").run?.(ctx());
    expect(nav.goto).toHaveBeenLastCalledWith("/templates/t1");
    await byId(items, "source.open:src1").run?.(ctx());
    expect(nav.goto).toHaveBeenLastCalledWith("/sources?source=src1");
    await byId(items, "settings.section:backups").run?.(ctx());
    expect(api.openPreferences).toHaveBeenLastCalledWith("backups");
    await byId(items, "screen.control:s1").run?.(ctx());
    expect(uiState.value?.control.selectedScreenId).toBe("s1");
    expect(nav.goto).toHaveBeenLastCalledWith("/control");
  });

  it("GO {cue} for cue flows (disarmed: disabled), calling runFlow; not when Control registered it", async () => {
    await loaded();
    let items = providerItems(ctx());
    const go = byId(items, "cue.go:f1");
    expect(titleOf(go)).toBe("GO Opening cue");
    expect(go.live).toBe(true);
    expect(go.section).toBe("live");
    expect(ids(items)).not.toContain("cue.go:f2");
    expect(byId(items, "cue.go:f3").disabled?.(ctx())).toBe("Disarmed");
    await go.run?.(ctx());
    expect(api.runFlow).toHaveBeenCalledWith("f1");
    // Control's own cue command (with its key) wins: never listed twice.
    cleanups.push(registerCommands([{ id: "cue.go:f1", title: "GO {cue}", section: "live", run: () => {} }]));
    items = providerItems(ctx());
    expect(ids(items)).not.toContain("cue.go:f1");
  });

  it("slides: Next / Previous for sources with auto-advance, through stepSource", async () => {
    await loaded();
    const items = providerItems(ctx({ route: "control" }));
    expect(ids(items)).not.toContain("source.next:src1");
    const next = byId(items, "source.next:slides");
    expect(titleOf(next)).toBe("Next slide in Sponsors");
    expect(next.rank?.(ctx({ route: "control" }))).toBe(10_000);
    expect(next.rank?.(ctx())).toBe(0);
    await next.run?.(ctx());
    expect(api.stepSource).toHaveBeenCalledWith("slides", { to: "by", delta: 1 });
    await byId(items, "source.prev:slides").run?.(ctx());
    expect(api.stepSource).toHaveBeenLastCalledWith("slides", { to: "by", delta: -1 });
  });

  it("schedule items: Start now (not NOW or cancelled) and Set as next (not NOW, previous or next)", async () => {
    await loaded();
    const items = providerItems(ctx());
    const sched = ids(items).filter((i) => i.startsWith("schedule."));
    expect(sched).toEqual([
      "schedule.start:sch/i0",
      "schedule.start:sch/i2",
      "schedule.start:sch/i3",
      "schedule.next:sch/i3",
    ]);
    expect(titleOf(byId(items, "schedule.start:sch/i2"))).toBe("Start Panel now");
    expect(titleOf(byId(items, "schedule.next:sch/i3"))).toBe("Set Lunch as next");
    await byId(items, "schedule.start:sch/i2").run?.(ctx());
    expect(api.scheduleAction).toHaveBeenCalledWith("sch", { do: "startNow", itemId: "i2" });
    await byId(items, "schedule.next:sch/i3").run?.(ctx());
    expect(api.scheduleAction).toHaveBeenLastCalledWith("sch", { do: "setNext", itemId: "i3", start: null });
  });

  it("a failing list keeps its last items; the others still load", async () => {
    api.listTemplates.mockRejectedValue(new Error("no backend"));
    api.listFlows.mockResolvedValue([flow("f9", "New", false)]);
    await loaded();
    await vi.waitFor(() => expect(ids(providerItems(ctx()))).toContain("flow.open:f9"));
    expect(ids(providerItems(ctx()))).toContain("template.edit:t1");
  });
});

// ---- Control --------------------------------------------------------------------------------------

function fakeCtl(screens: LiveScreenView[], studio = false) {
  return {
    capturing: null as string | null,
    visible: screens,
    studio: { on: studio },
    take: vi.fn(async () => {}),
    clear: vi.fn(async () => {}),
    toggle: vi.fn(async () => {}),
    clearScreen: vi.fn(async () => {}),
    clearEvery: vi.fn(async () => {}),
    panic: vi.fn(async () => {}),
    flows: { run: vi.fn(async () => {}) },
  };
}
const asCtl = (c: ReturnType<typeof fakeCtl>) => c as unknown as ControlCtl;

describe("Control's live items", () => {
  const screens = () => [
    view("s1", "Main", [layer("l1", "Opening keynote"), layer("l2", "Guest", { onAir: true, pending: true })]),
    view("s2", "Side", [layer("l3", "Bug", { onAir: true })]),
  ];

  it("Take / Clear / Update per layer (front first), Clear {screen}; the screen in focus first and unqualified", async () => {
    const ctl = fakeCtl(screens());
    const p = controlProvider(asCtl(ctl));
    const c = ctx({ route: "control", screenId: "s2" });
    const items = p.items(c);
    expect(ids(items)).toEqual([
      "live.clear:s2/l3",
      "live.clearScreen:s2",
      "live.update:s1/l2",
      "live.clear:s1/l2",
      "live.take:s1/l1",
      "live.clearScreen:s1",
    ]);
    expect(titleOf(byId(items, "live.clear:s2/l3"))).toBe("Clear Bug");
    expect(titleOf(byId(items, "live.take:s1/l1"))).toBe("Take Opening keynote (Main)");
    expect(titleOf(byId(items, "live.clearScreen:s1"))).toBe("Clear Main");
    expect(items.every((i) => i.live && i.section === "live" && !i.confirm)).toBe(true);
    expect(byId(items, "live.clear:s2/l3").rank?.(c)).toBe(11_000);
    expect(byId(items, "live.take:s1/l1").rank?.(c)).toBe(10_000);
    // Only in Control, never while "Set hotkey" waits for a key.
    expect(p.items(ctx({ screenId: "s2" }))).toEqual([]);
    ctl.capturing = "s1/l1";
    expect(p.items(c)).toEqual([]);
  });

  it("each item calls the action Control's buttons call", async () => {
    const ctl = fakeCtl(screens());
    const items = controlProvider(asCtl(ctl)).items(ctx({ route: "control", screenId: "s1" }));
    await byId(items, "live.take:s1/l1").run?.(ctx());
    expect(ctl.take).toHaveBeenCalledWith("s1", "l1");
    await byId(items, "live.update:s1/l2").run?.(ctx());
    expect(ctl.take).toHaveBeenLastCalledWith("s1", "l2");
    await byId(items, "live.clear:s1/l2").run?.(ctx());
    expect(ctl.clear).toHaveBeenCalledWith("s1", "l2");
    await byId(items, "live.clearScreen:s2").run?.(ctx());
    expect(ctl.clearScreen).toHaveBeenCalledWith("s2");
  });

  it("Studio mode: Cue {layer} (the toggle) instead of Take / Clear / Update", () => {
    const ctl = fakeCtl(screens(), true);
    const items = controlProvider(asCtl(ctl)).items(ctx({ route: "control", screenId: "s1" }));
    expect(ids(items).filter((i) => !i.startsWith("live.clearScreen"))).toEqual(["live.cue:s1/l2", "live.cue:s1/l1", "live.cue:s2/l3"]);
    void byId(items, "live.cue:s1/l1").run?.(ctx());
    expect(ctl.toggle).toHaveBeenCalledWith("s1", "l1");
  });

  it("registered commands: PANIC and Clear all screens confirm; layer keys are hidden toggles", () => {
    const ctl = fakeCtl(screens());
    const cmds = controlCommands(asCtl(ctl), [{ screenId: "s1", screen: "Main", layerId: "l1", name: "Opening keynote" }], [{ id: "f1", label: "Opening" }]);
    expect(byId(cmds, "live.panic").confirm).toBe(true);
    const all = byId(cmds, "live.clearAll");
    expect(all.confirm).toBe(true);
    expect(titleOf(all)).toBe("Clear all screens");
    void all.run?.(ctx());
    expect(ctl.clearEvery).toHaveBeenCalled();
    const toggle = byId(cmds, "live.toggle:s1/l1");
    expect(toggle.hidden).toBe(true);
    expect(titleOf(toggle)).toBe("Take or clear Opening keynote");
    // Hidden commands are never listed.
    const listed = search("", cmds, ctx({ route: "control" }), []).map((m) => m.command.id);
    expect(listed).not.toContain("live.toggle:s1/l1");
    expect(listed).toContain("cue.go:f1");
  });
});

// ---- Edit section and follow-ups ------------------------------------------------------------------

describe("Edit section", () => {
  it("comes after Create; layout and flow editor commands are in it", () => {
    expect(sectionOrder(ctx())).toEqual(["goto", "create", "edit", "items", "live", "settings"]);
    expect(sectionOrder(ctx({ route: "control" }))).toEqual(["live", "goto", "create", "edit", "items", "settings"]);
    const noop = () => {};
    const editor = editorCommands({
      view: () => ({}) as never,
      setView: noop,
      selectionSize: () => 1,
      layerCount: () => 1,
      hasGuides: () => true,
      selectAll: noop,
      align: noop,
      distribute: noop,
      clearGuides: noop,
      toggleLock: noop,
      editSafeAreas: noop,
    });
    expect(editor.every((c) => c.section === "edit" && c.keyScope === "screenEditor")).toBe(true);
    const flowCmds = flowEditorCommands({
      deploy: noop,
      deployBlocked: () => null,
      undo: noop,
      redo: noop,
      canUndo: () => true,
      canRedo: () => true,
      selectAll: noop,
      fit: noop,
    });
    expect(flowCmds.every((c) => c.section === "edit" && c.keyScope === "flowEditor")).toBe(true);
    const groups = search("", [...editor, { id: "nav.x", title: "Go to X", section: "goto", run: noop }], ctx({ route: "screenEditor" }), []);
    expect(groups.map((m) => m.section).filter((s, i, a) => a.indexOf(s) === i)).toEqual(["goto", "edit"]);
  });

  it("the layout editor: Edit {layer} selects it; Rename screen… validates like the name field", async () => {
    const select = vi.fn();
    const rename = vi.fn();
    const items = {
      layers: () => [
        { id: "l1", name: "Lower third" },
        { id: "l2", name: "Bug" },
      ],
      select,
      name: () => "Main",
      rename,
    };
    const p = editorProvider(items);
    expect(p.items(ctx({ route: "control" }))).toEqual([]);
    const list = p.items(ctx({ route: "screenEditor" }));
    expect(ids(list)).toEqual(["editor.layer:l1", "editor.layer:l2"]);
    expect(titleOf(list[1])).toBe("Edit Bug");
    await list[1].run?.(ctx());
    expect(select).toHaveBeenCalledWith("l2");

    const cmd = renameCommand(items);
    expect(cmd.id).toBe("screen.rename");
    expect(cmd.title).toBe("Rename screen…");
    expect(cmd.section).toBe("edit");
    const next = cmd.next;
    if (next?.kind !== "text") throw new Error("text step");
    expect(next.value?.(ctx())).toBe("Main");
    expect(next.validate?.("  ")).toBe("Enter a name.");
    expect(next.validate?.("x".repeat(200))).toMatch(/^Up to \d+ characters\.$/);
    expect(next.validate?.("Side")).toBeNull();
    await next.run("  Side  ", ctx());
    expect(rename).toHaveBeenCalledWith("Side");
  });

  it("Set variant…: one pick of Layer → Variant pairs (not the shown one), running the switch", async () => {
    const run = vi.fn();
    const layers = [
      { screenId: "s1", layerId: "l1", name: "Lower third", current: "full", variants: [{ id: "full", name: "Full" }, { id: "compact", name: "Compact" }] },
      { screenId: "s1", layerId: "l2", name: "Bug", current: "a", variants: [{ id: "a", name: "A" }] },
    ];
    const cmds = variantCommands(layers, { section: "live", keyScope: "control" }, run);
    const set = byId(cmds, "variant.set");
    expect(set.title).toBe("Set variant…");
    const next = set.next;
    if (next?.kind !== "pick") throw new Error("pick step");
    const opts = next.options(ctx());
    expect(opts.map((o) => o.title)).toEqual(["Lower third → Compact"]);
    await next.run(opts[0].id, ctx());
    expect(run).toHaveBeenCalledWith(layers[0], "compact");
    await next.run("not json", ctx());
    expect(run).toHaveBeenCalledTimes(1);
    // No layer with several variants: no command.
    expect(variantCommands([layers[1]], { section: "live", keyScope: "control" }, run)).toEqual([]);
  });

  it("prepare skips hidden commands", () => {
    const entries = prepare([{ id: "a", title: "A", section: "live", hidden: true, run: () => {} }], ctx());
    expect(entries).toEqual([]);
  });
});


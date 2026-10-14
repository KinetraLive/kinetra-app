import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  FlowInfo,
  FlowNode,
  LiveLayerView,
  LiveScreenView,
  NewFlow,
  Project,
  SaveFlow,
  ServerStatus,
  SourceSummary,
  UiState,
} from "$lib/types";

// ---- Mocks --------------------------------------------------------------------------------------

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Handler>() }));
const api = vi.hoisted(() => ({
  serverStatus: vi.fn(),
  listScreens: vi.fn(),
  getScreen: vi.fn(),
  getTemplate: vi.fn(),
  listSources: vi.fn(),
  getSchedule: vi.fn(),
  listTemplates: vi.fn(),
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  on: vi.fn(),
  listRecipeInstances: vi.fn(),
  listFlows: vi.fn(),
  getFlow: vi.fn(),
  runFlow: vi.fn(),
  stopFlow: vi.fn(),
  setFlowArmed: vi.fn(),
  createFlow: vi.fn(),
  saveFlow: vi.fn(),
  duplicateFlow: vi.fn(),
  deleteFlow: vi.fn(),
  listFlowTemplates: vi.fn(),
  addFlowTemplate: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e
      ? e
      : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/screens", () => api);
vi.mock("$lib/api/sources", () => api);
vi.mock("$lib/api/templates", () => api);
vi.mock("$lib/api/automation", () => api);
vi.mock("$lib/api/recipes", () => api);
vi.mock("$lib/api/flows", () => api);
const navigation = vi.hoisted(() => ({ goto: vi.fn(), afterNavigate: vi.fn() }));
vi.mock("$app/navigation", () => navigation);
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/control") } }));

class FakeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
}
vi.stubGlobal("IntersectionObserver", FakeObserver);
vi.stubGlobal("ResizeObserver", FakeObserver);

import ControlPage from "../../../routes/(app)/control/+page.svelte";
import { live } from "$lib/stores/live.svelte";
import { project } from "$lib/stores/project.svelte";
import { server } from "$lib/stores/server.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { uiState } from "$lib/stores/uiState.svelte";
import { toLinear } from "./linear";

// ---- Fixtures -----------------------------------------------------------------------------------

const running = { state: "running", port: 7770, url: "http://127.0.0.1:7770", lanUrl: null } satisfies ServerStatus;

const current: Project = {
  path: "C:\\shows\\flows",
  name: "Flows Show",
  createdAt: "2026-01-02T10:00:00Z",
  settings: { port: 7770, lanAccess: false, defaultResolution: { width: 1920, height: 1080 } },
};

const ui = (hotkeys: Record<string, string> = {}): UiState => ({
  sidebarPinned: false,
  control: {
    view: "single",
    gridColumns: 2,
    selectedScreenId: "s1",
    hiddenScreenIds: [],
    liveEdit: "onTake",
    hotkeys,
    automationOff: false,
  },
});

const layer = (id: string, name: string, patch: Partial<LiveLayerView> = {}): LiveLayerView => ({
  id,
  name,
  onAir: false,
  pending: false,
  fields: {},
  fieldDefs: [],
  ...patch,
});

const main: LiveScreenView = {
  screen: { id: "s1", name: "Main", width: 1920, height: 1080, background: "transparent" },
  layers: [
    layer("board", "Board"),
    layer("nn", "Now & Next", {
      fieldDefs: [
        { key: "title", label: "Title", type: "text", default: "" },
        { key: "left", label: "Left", type: "timer", default: "" },
      ],
    }),
  ],
};

function source(id: string, name: string, kind: "text" | "schedule"): SourceSummary {
  return {
    id,
    name,
    kind,
    rowCount: 2,
    index: 0,
    current: {},
    columns: [],
    usedBy: [],
    updatedAt: "",
    auto: null,
    nextAt: null,
    nowId: null,
    nextId: null,
  };
}

const n = (id: string, kind: object, x = 0) => ({ id, position: { x, y: 0 }, ...kind }) as FlowNode;
const ev = (id: string, from: string, port: string, to: string) => ({
  id,
  from: { node: from, port },
  to: { node: to, port: "in" },
  kind: "event" as const,
});

function info(patch: Partial<FlowInfo>): FlowInfo {
  return {
    id: "x",
    name: "x",
    armed: true,
    retrigger: "restart",
    origin: null,
    nodes: [],
    edges: [],
    lastFiredAt: null,
    updatedAt: "2026-10-04T08:00:00Z",
    warnings: [],
    running: false,
    readOnly: false,
    ...patch,
  };
}

/** A rule of the screen (no automation): when Board goes on air, take Now & Next, wait 4 s, clear it. */
const opener = info({
  id: "f1",
  name: "Opener",
  nodes: [
    n("t1", { type: "trigger.layer", config: { screenId: "s1", layerId: "board", onAir: true } }),
    n("a1", { type: "action.take", config: { screenId: "s1", layerId: "nn", forMs: null } }, 240),
    n("d1", { type: "logic.delay", config: { ms: 4000 } }, 480),
    n("c1", { type: "action.clear", config: { screenId: "s1", layerId: "nn" } }, 720),
  ],
  edges: [ev("e1", "t1", "fired", "a1"), ev("e2", "a1", "done", "d1"), ev("e3", "d1", "out", "c1")],
  lastFiredAt: new Date(Date.now() - 125_000).toISOString(),
});

/** A recipe's rule, disarmed, with a warning. */
const strap = info({
  id: "f2",
  name: "Strap on NOW",
  armed: false,
  origin: "rcp1",
  nodes: [
    n("t2", { type: "trigger.itemStatus", config: { sourceId: "sched", status: "now", itemId: null } }),
    n("a2", { type: "action.take", config: { screenId: "s1", layerId: "nn", forMs: 10_000 } }, 240),
  ],
  edges: [ev("e4", "t2", "fired", "a2")],
  warnings: [
    { node: "a2", message: "Layer gone" },
    { node: "a2", message: "Field gone" },
  ],
});

/** Not a simple chain: a Branch. */
const branchy = info({
  id: "f3",
  name: "Branchy",
  nodes: [
    n("t3", { type: "trigger.layer", config: { screenId: "s1", layerId: "board", onAir: false } }),
    n("b3", { type: "logic.branch", config: {} }, 240),
    n("a3", { type: "action.clearScreen", config: { screenId: "s1" } }, 480),
  ],
  edges: [ev("e5", "t3", "fired", "b3"), ev("e6", "b3", "then", "a3")],
});

const flush = async () => {
  for (let i = 0; i < 6; i++) {
    await Promise.resolve();
    await tick();
  }
};

async function mount(flows: FlowInfo[] = [opener, strap, branchy], hotkeys: Record<string, string> = {}) {
  api.listFlows.mockResolvedValue(flows);
  uiState.set(ui(hotkeys));
  render(ControlPage);
  await flush();
  await flush();
}

const lastUi = () => (api.setUiState.mock.calls.at(-1)![0] as UiState).control;

/** The Automation strip with every row expanded (which rows are open is kept for the session). */
async function openFlows() {
  const region = await screen.findByRole("region", { name: "Automation" });
  await waitFor(() => expect(region.querySelector(".toggle, .empty")).toBeTruthy());
  await flush();
  for (const t of region.querySelectorAll<HTMLElement>(".toggle[aria-expanded=false]")) await fireEvent.click(t);
  await flush();
  return region;
}

/** "New rule" of the screen's own rules (Other rules), or of the empty state. */
const newRuleButton = (region: HTMLElement) =>
  within(region).queryByRole("button", { name: "New rule for Other rules" }) ?? within(region).getByRole("button", { name: "New rule" });

const row = (id: string) => document.querySelector<HTMLElement>(`[data-flow="${id}"]`)!;

async function menu(id: string, name: string) {
  await fireEvent.click(within(row(id)).getByRole("button", { name: `Actions for ${name}` }));
  return within(row(id));
}

beforeEach(() => {
  vi.clearAllMocks();
  events.handlers.clear();
  live.screens = new Map();
  server.status = null;
  api.on.mockImplementation((name: string, h: Handler) => {
    events.handlers.set(name, h);
    return () => events.handlers.delete(name);
  });
  api.serverStatus.mockResolvedValue(running);
  api.listScreens.mockResolvedValue([main]);
  api.getScreen.mockRejectedValue(new Error("no doc"));
  api.listSources.mockResolvedValue([source("sched", "Run of show", "schedule"), source("sp", "Sponsors", "text")]);
  api.getSchedule.mockResolvedValue({ id: "sched", name: "Run of show", kind: "schedule", items: [] });
  api.listTemplates.mockResolvedValue([]);
  api.setUiState.mockImplementation(async (v: UiState) => v);
  api.listRecipeInstances.mockResolvedValue([
    {
      id: "rcp1",
      recipeId: "event-schedule",
      name: "Main stage",
      panel: "schedule",
      sourceId: null,
      screenId: "s1",
      layerIds: [],
      templateIds: [],
      flowIds: ["f2"],
      createdAt: "",
    },
  ]);
  api.setFlowArmed.mockImplementation(async (id: string, armed: boolean) => ({
    ...[opener, strap, branchy].find((f) => f.id === id)!,
    armed,
  }));
  api.createFlow.mockImplementation(async (f: NewFlow) => info({ ...f, id: "new1" }));
  api.saveFlow.mockImplementation(async (f: SaveFlow) => info({ ...opener, ...f }));
  api.deleteFlow.mockResolvedValue(undefined);
  api.duplicateFlow.mockImplementation(async () => info({ ...opener, id: "f4", name: "Opener copy" }));
  project.current = current;
});

afterEach(() => {
  cleanup();
  toasts.clear();
});

// ---- Rule rows (Control's Automation strip) ---------------------------------------------------------

describe("rule rows", () => {
  it("under their automation or Other rules, with summaries and states; warnings aggregated per row", async () => {
    await mount();
    const region = await openFlows();
    const recipe = region.querySelector<HTMLElement>('[data-recipe="rcp1"]')!;
    const other = region.querySelector<HTMLElement>('[data-row="__other"]')!;
    expect(within(recipe).getByText("Strap on NOW")).toBeTruthy();
    expect(within(other).getByText("Opener")).toBeTruthy();
    expect(within(other).getByText("Branchy")).toBeTruthy();

    expect(row("f1").textContent).toContain("take Now & Next, wait 4 s, clear Now & Next");
    expect(row("f1").querySelector(".fired")?.textContent?.trim()).toBe("2 min ago");
    expect(row("f2").textContent).toContain("When item becomes NOW in Run of show then take Now & Next for 10 s");
    expect(row("f2").textContent).not.toContain("warning");
    expect(recipe.querySelector(".rhead")?.textContent).toContain("2 warnings");
    expect(within(recipe).getByText("Layer gone")).toBeTruthy();
    expect(row("f2").querySelector(".fired")?.textContent?.trim()).toBe("never");
    expect(row("f3").textContent).toContain("3 nodes");
  });

  it("the armed switch saves at once; a failure rolls back", async () => {
    await mount();
    await openFlows();
    const sw = within(row("f2")).getByRole("switch", { name: "Arm Strap on NOW" });
    expect(sw.getAttribute("aria-checked")).toBe("false");
    await fireEvent.click(sw);
    await flush();
    expect(api.setFlowArmed).toHaveBeenLastCalledWith("f2", true);
    expect(sw.getAttribute("aria-checked")).toBe("true");

    api.setFlowArmed.mockRejectedValueOnce(new Error("disk full"));
    await fireEvent.click(sw);
    await flush();
    expect(sw.getAttribute("aria-checked")).toBe("true");
    expect(toasts.list.at(-1)?.message).toBe("Couldn't disarm Strap on NOW: disk full");
  });

  it("Edit only for simple chains; Duplicate adds the copy", async () => {
    await mount();
    await openFlows();
    let m = await menu("f3", "Branchy");
    expect(m.queryByRole("menuitem", { name: "Edit" })).toBeNull();
    expect(m.getByRole("menuitem", { name: "Duplicate" })).toBeTruthy();
    await fireEvent.keyDown(document.activeElement ?? window, { key: "Escape" });

    m = await menu("f1", "Opener");
    await fireEvent.click(m.getByRole("menuitem", { name: "Duplicate" }));
    await flush();
    expect(api.duplicateFlow).toHaveBeenCalledWith("f1");
    expect(row("f4")).toBeTruthy();
  });

  it("Edit in Flows, for every rule (simple chains or not)", async () => {
    await mount();
    await openFlows();
    const m = await menu("f3", "Branchy");
    await fireEvent.click(m.getByRole("menuitem", { name: "Edit in Flows" }));
    expect(navigation.goto).toHaveBeenCalledWith("/flows/f3");
    const o = await menu("f1", "Opener");
    expect(o.getByRole("menuitem", { name: "Edit in Flows" })).toBeTruthy();
  });

  it("Delete asks first, then deletes the rule and its hotkey", async () => {
    await mount(undefined, { "flow/f1": "F5", "s1/nn": "F1" });
    await openFlows();
    await fireEvent.click((await menu("f1", "Opener")).getByRole("menuitem", { name: "Delete…" }));
    const dlg = await screen.findByRole("dialog", { name: "Delete Opener?" });
    await fireEvent.click(within(dlg).getByRole("button", { name: "Cancel" }));
    expect(api.deleteFlow).not.toHaveBeenCalled();

    await fireEvent.click((await menu("f1", "Opener")).getByRole("menuitem", { name: "Delete…" }));
    await fireEvent.click(within(await screen.findByRole("dialog", { name: "Delete Opener?" })).getByRole("button", { name: "Delete" }));
    await flush();
    expect(api.deleteFlow).toHaveBeenCalledWith("f1");
    expect(row("f1")).toBeNull();
    expect(lastUi().hotkeys).toEqual({ "s1/nn": "F1" });
  });

  it("a read-only rule can't be armed, edited or deleted; an invalid file's reason is in the row's warnings", async () => {
    const ro = info({ ...opener, id: "f5", name: "Future", readOnly: true });
    const bad = info({ ...strap, id: "f6", name: "Broken", origin: null, warnings: [{ node: null, message: "Not valid, so it doesn't run: bad port" }] });
    await mount([ro, bad]);
    const region = await openFlows();
    expect((within(row("f5")).getByRole("switch") as HTMLButtonElement).disabled).toBe(true);
    expect(row("f5").textContent).toContain("Read-only");
    const m = await menu("f5", "Future");
    expect((m.getByRole("menuitem", { name: "Edit" }) as HTMLButtonElement).disabled).toBe(true);
    expect((m.getByRole("menuitem", { name: "Delete…" }) as HTMLButtonElement).disabled).toBe(true);
    expect((m.getByRole("menuitem", { name: "Duplicate" }) as HTMLButtonElement).disabled).toBe(false);
    const warnings = within(region).getByRole("list", { name: "Warnings of Other rules" });
    expect(warnings.textContent).toContain("Not valid, so it doesn't run: bad port");
    expect(warnings.textContent).toContain("Affects Broken");
  });
});

// ---- Linear editor ------------------------------------------------------------------------------

describe("linear editor", () => {
  const dialog = (name: string) => screen.findByRole("dialog", { name });
  const change = async (el: HTMLElement, value: string) => {
    (el as HTMLInputElement | HTMLSelectElement).value = value;
    await fireEvent.change(el);
  };

  it("New rule: a time-of-day trigger and every kind of step, created disarmed", async () => {
    await mount([opener]);
    const region = await openFlows();
    await fireEvent.click(newRuleButton(region));
    const dlg = within(await dialog("New rule"));
    expect((dlg.getByLabelText("Name") as HTMLInputElement).value).toBe("Rule 1");
    await change(dlg.getByLabelText("Trigger"), "trigger.timeOfDay");
    // An invalid time stays in the box and blocks Save; "1830" is 18:30.
    const tod = dlg.getByLabelText("Time of day") as HTMLInputElement;
    await fireEvent.input(tod, { target: { value: "25:00" } });
    await fireEvent.blur(tod);
    expect(tod.value).toBe("25:00");
    expect(tod.getAttribute("aria-invalid")).toBe("true");
    expect(dlg.getByRole("button", { name: "Save" }).hasAttribute("disabled")).toBe(true);
    await fireEvent.input(tod, { target: { value: "1830" } });
    await fireEvent.blur(tod);
    expect(tod.value).toBe("18:30");
    expect(dlg.getByRole("button", { name: "Save" }).hasAttribute("disabled")).toBe(false);
    await fireEvent.click(dlg.getByRole("button", { name: "Sat" }));
    await fireEvent.click(dlg.getByRole("button", { name: "Sun" }));
    // No retrigger choice for rules.
    expect(dlg.queryByLabelText("When it runs again")).toBeNull();

    // Step 1 is a take of the selected screen's front layer; for 15 s.
    await change(dlg.getByLabelText("Step 1 seconds on air (0 = stays)"), "15");
    const add = async (type: string) => {
      await fireEvent.click(dlg.getByRole("button", { name: "Add step" }));
      const count = document.querySelectorAll("[data-step]").length;
      await change(dlg.getByLabelText(`Step ${count} type`), type);
      return count;
    };
    let i = await add("logic.delay");
    await change(dlg.getByLabelText(`Step ${i} seconds to wait`), "2.5");
    i = await add("action.setFields");
    await fireEvent.click(dlg.getByRole("button", { name: "Add field" }));
    await fireEvent.input(dlg.getByLabelText(`Step ${i} value of title`), { target: { value: "Doors" } });
    i = await add("action.setTimer");
    await change(dlg.getByLabelText(`Step ${i} timer action`), "set");
    await change(dlg.getByLabelText(`Step ${i} timer seconds`), "90");
    i = await add("action.stepSource");
    await change(dlg.getByLabelText(`Step ${i} direction`), "prev");
    i = await add("action.pauseSource");
    await change(dlg.getByLabelText(`Step ${i} pause or resume`), "toggle");
    i = await add("action.schedule");
    await change(dlg.getByLabelText(`Step ${i} item`), "next");
    await change(dlg.getByLabelText(`Step ${i} schedule action`), "delay");
    await change(dlg.getByLabelText(`Step ${i} delay minutes`), "10");
    i = await add("action.runFlow");
    // A run-flow step needs a flow: Save is blocked until one is chosen.
    expect(dlg.getByText("1 problem to fix")).toBeTruthy();
    expect((dlg.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
    await change(dlg.getByLabelText(`Step ${i} flow`), "f1");
    i = await add("action.log");
    await fireEvent.input(dlg.getByLabelText(`Step ${i} message`), { target: { value: "Doors open" } });
    i = await add("action.clearScreen");
    i = await add("action.clearAll");
    // Move the last step (clear all) up with Alt+↑.
    await fireEvent.keyDown(dlg.getByLabelText(`Step ${i} type`), { key: "ArrowUp", altKey: true });

    await fireEvent.click(dlg.getByRole("button", { name: "Save" }));
    await flush();
    const req = api.createFlow.mock.calls.at(-1)![0] as NewFlow;
    expect(req).toMatchObject({ name: "Rule 1", armed: false, retrigger: "restart" });
    const l = toLinear(req)!;
    expect(l.trigger).toMatchObject({ type: "trigger.timeOfDay", config: { at: "18:30", days: [5, 6] } });
    expect(l.steps.map((s) => ({ type: s.type, config: s.config }))).toEqual([
      { type: "action.take", config: { screenId: "s1", layerId: "nn", forMs: 15_000 } },
      { type: "logic.delay", config: { ms: 2500 } },
      { type: "action.setFields", config: { screenId: "s1", layerId: "nn", fields: { title: "Doors" } } },
      { type: "action.setTimer", config: { screenId: "s1", layerId: "nn", key: "left", action: { action: "set", ms: 90_000 } } },
      { type: "action.stepSource", config: { sourceId: "sp", step: { to: "by", delta: -1 } } },
      { type: "action.pauseSource", config: { sourceId: "sp", mode: "toggle" } },
      { type: "action.schedule", config: { sourceId: "sched", item: { pick: "next" }, op: { op: "delay", minutes: 10 } } },
      { type: "action.runFlow", config: { flowId: "f1", wait: false } },
      { type: "action.log", config: { message: "Doors open", toast: false } },
      { type: "action.clearAll", config: {} },
      { type: "action.clearScreen", config: { screenId: "s1" } },
    ]);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "New rule" })).toBeNull());
    expect(row("new1")).toBeTruthy();
  });

  it("a Set variant step lists the layer's variants and its transition; Save needs a variant", async () => {
    const look = (id: string, name: string) => ({ id, name, vars: {}, html: "", css: "" });
    api.getScreen.mockResolvedValue({ doc: { id: "s1", layers: [{ id: "nn", templateId: "t1" }] }, live: null, warnings: [] });
    api.getTemplate.mockResolvedValue({ id: "t1", defaultVariant: "full", variants: [look("full", "Full"), look("compact", "Compact")] });
    await mount([opener]);
    const region = await openFlows();
    await fireEvent.click(newRuleButton(region));
    const dlg = within(await dialog("New rule"));
    await change(dlg.getByLabelText("Step 1 type"), "action.setVariant");
    // The layer carries over from the Take; a variant must be chosen.
    expect((dlg.getByLabelText("Step 1 layer") as HTMLSelectElement).value).toBe("nn");
    expect(dlg.getByText("Choose a variant.")).toBeTruthy();
    expect((dlg.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
    const variant = dlg.getByLabelText("Step 1 variant") as HTMLSelectElement;
    await waitFor(() =>
      expect([...variant.options].map((o) => o.textContent)).toEqual(["Choose a variant", "Full (default)", "Compact"]),
    );
    await change(variant, "compact");
    await change(dlg.getByLabelText("Step 1 transition while on air"), "outIn");
    await fireEvent.click(dlg.getByRole("button", { name: "Save" }));
    await flush();
    const l = toLinear(api.createFlow.mock.calls.at(-1)![0] as NewFlow)!;
    expect(l.steps.map((s) => ({ type: s.type, config: s.config }))).toEqual([
      { type: "action.setVariant", config: { screenId: "s1", layerId: "nn", variant: "compact", transition: "outIn" } },
    ]);
  });

  it("a Take preview step: the screen and the transition (project default, or a kind with a duration)", async () => {
    await mount([opener]);
    const region = await openFlows();
    await fireEvent.click(newRuleButton(region));
    const dlg = within(await dialog("New rule"));
    await change(dlg.getByLabelText("Step 1 type"), "action.takePreview");
    // The screen carries over from the Take; the transition starts as the project default.
    expect((dlg.getByLabelText("Step 1 screen") as HTMLSelectElement).value).toBe("s1");
    const tr = dlg.getByLabelText("Step 1 transition") as HTMLSelectElement;
    expect(tr.value).toBe("default");
    expect(dlg.queryByLabelText("Step 1 crossfade duration")).toBeNull();
    await change(tr, "crossfade");
    await change(dlg.getByLabelText("Step 1 crossfade duration"), "1200");
    await fireEvent.click(dlg.getByRole("button", { name: "Save" }));
    await flush();
    const l = toLinear(api.createFlow.mock.calls.at(-1)![0] as NewFlow)!;
    expect(l.steps.map((s) => ({ type: s.type, config: s.config }))).toEqual([
      { type: "action.takePreview", config: { screenId: "s1", transition: { kind: "crossfade", ms: 1200 } } },
    ]);
  });

  it("each trigger kind builds its config", async () => {
    api.listRecipeInstances.mockResolvedValue([]);
    await mount([]);
    const region = await openFlows();
    await fireEvent.click(newRuleButton(region));
    const dlg = within(await dialog("New rule"));
    const trigger = async () => {
      await fireEvent.click(dlg.getByRole("button", { name: "Save" }));
      await flush();
      const t = toLinear(api.createFlow.mock.calls.at(-1)![0] as NewFlow)!.trigger;
      return { type: t.type, config: t.config };
    };
    api.createFlow.mockRejectedValue({ code: "invalid", message: "kept open" });
    // The default rule trigger: a schedule item becoming NOW.
    expect(await trigger()).toEqual({ type: "trigger.itemStatus", config: { sourceId: "sched", status: "now", itemId: null } });
    expect(dlg.getByRole("alert").textContent).toBe("kept open");
    await change(dlg.getByLabelText("Status"), "next");
    expect((await trigger()).config).toMatchObject({ status: "next" });
    await change(dlg.getByLabelText("Trigger"), "trigger.beforeNext");
    await change(dlg.getByLabelText("Minutes before"), "10");
    expect(await trigger()).toEqual({ type: "trigger.beforeNext", config: { sourceId: "sched", minutes: 10 } });
    await change(dlg.getByLabelText("Trigger"), "trigger.layer");
    await change(dlg.getByLabelText("On or off air"), "off");
    expect(await trigger()).toEqual({ type: "trigger.layer", config: { screenId: "s1", layerId: "nn", onAir: false } });
    await change(dlg.getByLabelText("Trigger"), "trigger.timerZero");
    await change(dlg.getByLabelText("Timer field"), "left");
    expect(await trigger()).toEqual({ type: "trigger.timerZero", config: { screenId: "s1", layerId: "nn", key: "left" } });
    await change(dlg.getByLabelText("Trigger"), "trigger.slideshowWrap");
    expect(await trigger()).toEqual({ type: "trigger.slideshowWrap", config: { sourceId: "sp" } });
    await change(dlg.getByLabelText("Trigger"), "trigger.sourceRow");
    expect(await trigger()).toEqual({ type: "trigger.sourceRow", config: { sourceId: "sp" } });
    await change(dlg.getByLabelText("Trigger"), "trigger.runByFlow");
    expect(await trigger()).toEqual({ type: "trigger.runByFlow", config: {} });
    await change(dlg.getByLabelText("Trigger"), "trigger.rest");
    expect(await trigger()).toEqual({ type: "trigger.rest", config: {} });
    expect(dlg.getByText("Companion or a script runs it through the Control API.")).toBeTruthy();
    await change(dlg.getByLabelText("Trigger"), "trigger.cue");
    await fireEvent.input(dlg.getByLabelText("Button label"), { target: { value: "Go" } });
    await change(dlg.getByLabelText("When it runs again"), "ignore");
    expect(await trigger()).toEqual({ type: "trigger.cue", config: { label: "Go" } });
    expect(api.createFlow.mock.calls.at(-1)![0]).toMatchObject({ armed: true, retrigger: "ignore" });
  });

  it("API value changed: an API source and optionally a column; the row summary reads it", async () => {
    api.listRecipeInstances.mockResolvedValue([]);
    api.listSources.mockResolvedValue([
      source("sched", "Run of show", "schedule"),
      source("sp", "Sponsors", "text"),
      {
        ...source("feed", "Scores", "text"),
        kind: "api",
        columns: [
          { key: "home", label: "Home" },
          { key: "score", label: "Score" },
        ],
      },
    ]);
    await mount([]);
    const region = await openFlows();
    await fireEvent.click(newRuleButton(region));
    const dlg = within(await dialog("New rule"));
    await change(dlg.getByLabelText("Trigger"), "trigger.apiChanged");
    const sourceSel = dlg.getByLabelText("Trigger API source") as HTMLSelectElement;
    // Only API sources to pick from.
    expect([...sourceSel.options].map((o) => o.textContent)).toEqual(["Scores"]);
    const colSel = dlg.getByLabelText("Trigger column") as HTMLSelectElement;
    expect([...colSel.options].map((o) => o.textContent)).toEqual(["Any row changes", "Home", "Score"]);
    await change(colSel, "score");
    await fireEvent.click(dlg.getByRole("button", { name: "Save" }));
    await flush();
    const created = api.createFlow.mock.calls.at(-1)![0] as NewFlow;
    expect(toLinear(created)!.trigger).toMatchObject({ type: "trigger.apiChanged", config: { sourceId: "feed", column: "score" } });
    expect(created.armed).toBe(false);
  });

  it("Edit saves the same flow (ids kept); the server's refusal shows inline and the editor stays", async () => {
    await mount();
    await openFlows();
    await fireEvent.click((await menu("f1", "Opener")).getByRole("menuitem", { name: "Edit" }));
    const dlg = within(await dialog("Edit rule"));
    await fireEvent.input(dlg.getByLabelText("Name"), { target: { value: "  Opening  " } });
    await change(dlg.getByLabelText("Step 2 seconds to wait"), "6");
    await fireEvent.click(dlg.getByRole("button", { name: "Delete step 3" }));

    api.saveFlow.mockRejectedValueOnce({ code: "invalid", message: "At most 200 nodes" });
    await fireEvent.click(dlg.getByRole("button", { name: "Save" }));
    await flush();
    expect(dlg.getByRole("alert").textContent).toBe("At most 200 nodes");

    await fireEvent.click(dlg.getByRole("button", { name: "Save" }));
    await flush();
    const req = api.saveFlow.mock.calls.at(-1)![0] as SaveFlow;
    expect(req).toMatchObject({ id: "f1", name: "Opening", retrigger: "restart" });
    expect(req.nodes.map((x) => x.id)).toEqual(["t1", "a1", "d1"]);
    expect(req.edges.map((e) => e.id)).toEqual(["e1", "e2"]);
    expect(req.nodes[2]).toMatchObject({ type: "logic.delay", config: { ms: 6000 } });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Edit rule" })).toBeNull());
    expect(row("f1").textContent).toContain("Opening");
  });

  it("schedule steps: Move start needs a date and time; Make next takes an optional one", async () => {
    api.listRecipeInstances.mockResolvedValue([]);
    await mount([]);
    const region = await openFlows();
    await fireEvent.click(newRuleButton(region));
    const dlg = within(await dialog("New rule"));
    await change(dlg.getByLabelText("Step 1 type"), "action.schedule");
    await change(dlg.getByLabelText("Step 1 schedule action"), "setStart");
    expect(dlg.getByText("Enter the new start date and time.")).toBeTruthy();
    const save = dlg.getByRole("button", { name: "Save" }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    await fireEvent.input(dlg.getByLabelText("Step 1 new start"), { target: { value: "2026-10-04T14:15" } });
    expect(save.disabled).toBe(false);
    await fireEvent.click(save);
    await flush();
    let step = toLinear(api.createFlow.mock.calls.at(-1)![0] as NewFlow)!.steps[0];
    expect(step.config).toMatchObject({ item: { pick: "now" }, op: { op: "setStart", start: "2026-10-04T14:15" } });

    await fireEvent.click(newRuleButton(region));
    const dlg2 = within(await dialog("New rule"));
    await change(dlg2.getByLabelText("Step 1 type"), "action.schedule");
    await change(dlg2.getByLabelText("Step 1 item"), "next");
    await change(dlg2.getByLabelText("Step 1 schedule action"), "setNext");
    // Empty: keeps its time.
    expect((dlg2.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(false);
    await fireEvent.input(dlg2.getByLabelText("Step 1 start (empty keeps its time)"), { target: { value: "2026-10-04T16:00" } });
    await fireEvent.click(dlg2.getByRole("button", { name: "Save" }));
    await flush();
    step = toLinear(api.createFlow.mock.calls.at(-1)![0] as NewFlow)!.steps[0];
    expect(step.config).toMatchObject({ item: { pick: "next" }, op: { op: "setNext", start: "2026-10-04T16:00" } });
  });

  it("Cancel discards; a missing target is a warning, not an error", async () => {
    const gone = info({
      ...opener,
      nodes: opener.nodes.map((x) => (x.id === "a1" ? n("a1", { type: "action.take", config: { screenId: "s1", layerId: "old", forMs: null } }, 240) : x)),
    });
    await mount([gone]);
    await openFlows();
    await fireEvent.click((await menu("f1", "Opener")).getByRole("menuitem", { name: "Edit" }));
    const dlg = within(await dialog("Edit rule"));
    expect(dlg.getByText("This layer no longer exists; skipped.")).toBeTruthy();
    expect(dlg.getByText("1 target is missing and will be skipped")).toBeTruthy();
    expect((dlg.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(false);
    await fireEvent.click(dlg.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Edit rule" })).toBeNull());
    expect(api.saveFlow).not.toHaveBeenCalled();
  });
});

// ---- Run flow and Stop flow steps -------------------------------------------------------------------
describe("linear editor: Run flow and Stop flow", () => {
  const change = async (el: HTMLElement, value: string) => {
    (el as HTMLInputElement | HTMLSelectElement).value = value;
    await fireEvent.change(el);
  };
  const texts = (el: HTMLElement) => [...(el as HTMLSelectElement).options].map((o) => o.textContent);

  it("Run flow with no other flow points at Repeat; Stop flow can stop this flow", async () => {
    await mount([opener]);
    await openFlows();
    await fireEvent.click((await menu("f1", "Opener")).getByRole("menuitem", { name: "Edit" }));
    const dlg = within(await screen.findByRole("dialog", { name: "Edit rule" }));
    await change(dlg.getByLabelText("Step 3 type"), "action.runFlow");
    const run = dlg.getByLabelText("Step 3 flow") as HTMLSelectElement;
    expect(texts(run)).toEqual(["No other flows — use the Repeat node to loop this flow."]);
    expect(run.options[0].disabled).toBe(true);

    await change(dlg.getByLabelText("Step 3 type"), "action.stopFlow");
    const stop = dlg.getByLabelText("Step 3 flow to stop") as HTMLSelectElement;
    expect(texts(stop)).toEqual(["Choose a flow", "This flow"]);
    await change(stop, "f1");
    const save = dlg.getByRole("button", { name: "Save" }) as HTMLButtonElement;
    expect(save.disabled).toBe(false);
    await fireEvent.click(save);
    await flush();
    const req = api.saveFlow.mock.calls.at(-1)![0] as SaveFlow;
    expect(req.nodes[3]).toMatchObject({ type: "action.stopFlow", config: { flowId: "f1" } });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Edit rule" })).toBeNull());
    expect(row("f1").textContent).toContain("stop this flow");
  });

  it("a Stop loop cue is two nodes: the cue and a Stop flow of the loop", async () => {
    api.listRecipeInstances.mockResolvedValue([]);
    await mount([opener]);
    const region = await openFlows();
    await fireEvent.click(newRuleButton(region));
    const dlg = within(await screen.findByRole("dialog", { name: "New rule" }));
    await change(dlg.getByLabelText("Trigger"), "trigger.cue");
    await change(dlg.getByLabelText("Step 1 type"), "action.stopFlow");
    const stop = dlg.getByLabelText("Step 1 flow to stop") as HTMLSelectElement;
    // A new flow has no id yet: only the others.
    expect(texts(stop)).toEqual(["Choose a flow", "Opener"]);
    expect(dlg.getByText("Choose a flow.")).toBeTruthy();
    await change(stop, "f1");
    await fireEvent.click(dlg.getByRole("button", { name: "Save" }));
    await flush();
    const req = api.createFlow.mock.calls.at(-1)![0] as NewFlow;
    expect(req.nodes.map((x) => x.type)).toEqual(["trigger.cue", "action.stopFlow"]);
    expect(req.nodes[1].config).toEqual({ flowId: "f1" });
    expect(req.edges).toHaveLength(1);
  });
});

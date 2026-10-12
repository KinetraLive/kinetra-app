import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  FlowInfo,
  LiveScreenView,
  RecipeInfo,
  RecipeInstance,
  ScheduleDoc,
  ScheduleView,
  ServerStatus,
  SourceSummary,
} from "$lib/types";

// ---- Mocks --------------------------------------------------------------------------------------

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Handler>() }));
const api = vi.hoisted(() => ({
  on: vi.fn(),
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  serverStatus: vi.fn(),
  listScreens: vi.fn(),
  setAutoAdvance: vi.fn(),
  stepSource: vi.fn(),
  listSources: vi.fn(),
  getSchedule: vi.fn(),
  listRecipes: vi.fn(),
  listRecipeInstances: vi.fn(),
  installRecipe: vi.fn(),
  removeRecipe: vi.fn(),
  relinkRecipeSource: vi.fn(),
  listFlows: vi.fn(),
  setFlowArmed: vi.fn(),
  duplicateFlow: vi.fn(),
  deleteFlow: vi.fn(),
  createFlow: vi.fn(),
  scheduleView: vi.fn(),
  scheduleAction: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) => ({ code: "internal", message: e instanceof Error ? e.message : String(e) }),
}));
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/screens", () => api);
vi.mock("$lib/api/sources", () => api);
vi.mock("$lib/api/templates", () => api);
vi.mock("$lib/api/automation", () => api);
vi.mock("$lib/api/recipes", () => api);
vi.mock("$lib/api/flows", () => api);
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));

import Automation from "./Automation.svelte";
import { collapseAll } from "./recipes.svelte";
import LinearEditor from "$lib/components/flows/LinearEditor.svelte";
import { toLinear } from "$lib/components/flows/linear";
import type { NewFlow } from "$lib/types";
import Menu from "$lib/components/control/Menu.svelte";
import { ControlCtl } from "$lib/components/control/control.svelte";
import { live } from "$lib/stores/live.svelte";
import { server } from "$lib/stores/server.svelte";
import { toasts } from "$lib/stores/toasts.svelte";

// ---- Fixtures -----------------------------------------------------------------------------------

const running = { state: "running", port: 7770, url: "http://127.0.0.1:7770", lanUrl: null } satisfies ServerStatus;

function screenView(id: string, name: string): LiveScreenView {
  return {
    screen: { id, name, width: 1920, height: 1080, background: "transparent" },
    layers: [
      {
        id: "nn",
        name: "Now & Next",
        onAir: false,
        pending: false,
        fields: {},
        fieldDefs: [
          { key: "title", label: "Title", type: "text", default: "" },
          { key: "left", label: "Left", type: "timer", default: "" },
        ],
      },
      { id: "board", name: "Board", onAir: false, pending: false, fields: {}, fieldDefs: [] },
    ],
  };
}

function summary(id: string, name: string, kind: "text" | "schedule"): SourceSummary {
  return {
    id,
    name,
    kind,
    rowCount: 2,
    index: 0,
    current: {},
    columns: [{ key: "name", label: "Name" }],
    usedBy: [],
    updatedAt: "2026-01-01T00:00:00Z",
    auto: null,
    nextAt: null,
    nowId: null,
    nextId: null,
  };
}

const flow: FlowInfo = {
  id: "fl1",
  name: "Strap on NOW",
  armed: true,
  retrigger: "restart",
  origin: "r-ev",
  nodes: [
    { id: "t", type: "trigger.itemStatus", config: { sourceId: "sched", status: "now", itemId: null }, position: { x: 0, y: 0 } },
    { id: "a", type: "action.take", config: { screenId: "s1", layerId: "nn", forMs: 10_000 }, position: { x: 240, y: 0 } },
  ],
  edges: [{ id: "e", from: { node: "t", port: "fired" }, to: { node: "a", port: "in" }, kind: "event" }],
  lastFiredAt: null,
  updatedAt: "2026-10-04T08:00:00Z",
  warnings: [],
  running: false,
  readOnly: false,
};
/** A flow of the project (no origin): not listed in recipe panels. */
const own: FlowInfo = { ...flow, id: "fl2", name: "Own flow", origin: null };

const eventInst: RecipeInstance = {
  id: "r-ev",
  recipeId: "event-schedule",
  name: "Event schedule",
  panel: "schedule",
  sourceId: "sched",
  screenId: "s1",
  layerIds: ["nn", "board"],
  templateIds: ["t1", "t2"],
  flowIds: ["fl1"],
  createdAt: "2026-10-04T08:00:00Z",
};

const slideInst: RecipeInstance = {
  id: "r-sl",
  recipeId: "sponsor-slideshow",
  name: "Sponsors",
  panel: "slideshow",
  sourceId: "sp",
  screenId: "s1",
  layerIds: ["board"],
  templateIds: ["t3"],
  flowIds: [],
  createdAt: "2026-10-04T08:00:00Z",
};

const gallery: RecipeInfo[] = [
  {
    id: "sponsor-slideshow",
    name: "Sponsor slideshow",
    description: "Logos and names",
    panel: "slideshow",
    version: 1,
    templates: ["classic-bar"],
  },
  {
    id: "event-schedule",
    name: "Event schedule",
    description: "NOW and UP NEXT",
    panel: "schedule",
    version: 1,
    templates: ["news-strap", "corner-bug"],
  },
];

const doc: ScheduleDoc = {
  id: "sched",
  name: "Run of show",
  kind: "schedule",
  items: [],
  ripple: false,
  nextOverride: null,
  labels: { now: "NOW", next: "UP NEXT", delayed: "DELAYED", previous: "PREVIOUS", cancelled: "CANCELLED", upcoming: "" },
  updatedAt: "2026-01-01T00:00:00Z",
};
const emptyView: ScheduleView = { sourceId: "sched", items: [], nowId: null, nextId: null, computedAt: Date.now(), canUndo: false };

const flush = async () => {
  for (let i = 0; i < 4; i++) {
    await Promise.resolve();
    await tick();
  }
};

async function mount(
  instances: RecipeInstance[] = [eventInst, slideInst],
  flows: FlowInfo[] = [flow, own],
  more: SourceSummary[] = [],
) {
  api.listRecipeInstances.mockResolvedValue(instances);
  live.screens = new Map([
    ["s1", screenView("s1", "Main")],
    ["s2", screenView("s2", "Side")],
  ]);
  const ctl = new ControlCtl();
  live.loaded = true;
  ctl.sources = [summary("sched", "Run of show", "schedule"), summary("sp", "Sponsors", "text"), ...more];
  ctl.sourcesLoaded = true;
  ctl.flows.list = flows;
  ctl.flows.loaded = true;
  api.listFlows.mockResolvedValue(flows);
  render(Automation, { props: { ctl, Menu } });
  await flush();
  return ctl;
}

const panel = (id: string) => document.querySelector<HTMLElement>(`[data-recipe="${id}"]`)!;
/** Text with the separators normalised (" · "). */
const txt = (el: Element) => (el.textContent ?? "").replace(/\s+/g, " ").replace(/\s*·\s*/g, " · ");

beforeEach(() => {
  vi.clearAllMocks();
  collapseAll();
  events.handlers.clear();
  api.on.mockImplementation((name: string, h: Handler) => {
    events.handlers.set(name, h);
    return () => events.handlers.delete(name);
  });
  api.serverStatus.mockResolvedValue(running);
  server.status = running;
  api.listScreens.mockResolvedValue([screenView("s1", "Main"), screenView("s2", "Side")]);
  api.listSources.mockResolvedValue([]);
  api.getSchedule.mockResolvedValue(doc);
  api.scheduleView.mockResolvedValue(emptyView);
  api.listRecipes.mockResolvedValue(gallery);
  api.listFlows.mockResolvedValue([flow, own]);
  api.setFlowArmed.mockImplementation(async (id: string, armed: boolean) => ({ ...flow, id, armed }));
  api.removeRecipe.mockResolvedValue(undefined);
});

afterEach(() => {
  cleanup();
  toasts.clear();
});

// ---- Strip --------------------------------------------------------------------------------------

const expand = async (id: string) => {
  await fireEvent.click(panel(id).querySelector<HTMLElement>(".toggle[aria-expanded=false]")!);
  await flush();
};
const strip = () => document.body;

// The strip loads its panels and the gallery as lazy chunks on first use. Load them once here, so a
// test never waits on a chunk being transformed (over the 1 s waits on a busy machine, in a full run).
beforeAll(async () => {
  const lazy = await import("./lazy");
  await Promise.all([lazy.loadSchedulePanel(), lazy.loadSlideshowPanel(), lazy.loadGallery()]);
}, 30_000);

describe("automation strip", () => {
  it("one collapsed row per automation of the screen, with its summary; expanding shows its live panel and rules", async () => {
    await mount();
    expect(txt(panel("r-ev"))).toContain("Event schedule · 1 rule · 1 armed");
    expect(txt(panel("r-sl"))).toContain("Sponsors · 0 rules");
    // Collapsed: no live panel, no rules.
    expect(api.scheduleView).not.toHaveBeenCalled();
    expect(within(panel("r-ev")).queryByText("Strap on NOW")).toBeNull();

    await expand("r-ev");
    await waitFor(() => expect(api.scheduleView).toHaveBeenCalledWith("sched"));
    expect(within(panel("r-ev")).getByText("Strap on NOW")).toBeTruthy();
    expect(within(panel("r-ev")).getByText("When item becomes NOW in Run of show then take Now & Next for 10 s")).toBeTruthy();
    expect(within(panel("r-ev")).getByText("never")).toBeTruthy();
    expect(within(panel("r-ev")).queryByText("Own flow")).toBeNull();

    await expand("r-sl");
    await waitFor(() => expect(within(panel("r-sl")).getByRole("button", { name: "Play" })).toBeTruthy());
    expect(within(panel("r-sl")).getByText("No rules yet.")).toBeTruthy();
    const toggle = panel("r-sl").querySelector<HTMLElement>(".toggle[aria-expanded=true]")!;
    await fireEvent.click(toggle);
    expect(within(panel("r-sl")).queryByRole("button", { name: "Play" })).toBeNull();
  });

  it("the screen's own rules are listed once, under Other rules; cues are not listed", async () => {
    const cue: FlowInfo = { ...own, id: "cue1", name: "A cue", nodes: [{ ...own.nodes[0], type: "trigger.cue", config: { label: null } } as FlowInfo["nodes"][number], own.nodes[1]] };
    await mount([eventInst, slideInst], [flow, own, cue]);
    const other = document.querySelector<HTMLElement>('[data-row="__other"]')!;
    expect(txt(other)).toContain("Other rules · 1 rule · 1 armed");
    await fireEvent.click(other.querySelector<HTMLElement>(".toggle")!);
    await expand("r-ev");
    await expand("r-sl");
    for (const id of ["fl1", "fl2"]) expect(document.querySelectorAll(`[data-flow="${id}"]`).length).toBe(1);
    expect(document.querySelector('[data-flow="cue1"]')).toBeNull();
    expect(within(other).getByText("Own flow")).toBeTruthy();
  });

  it("scoped to the selected screen: another screen's automation shows only when one of its rules aims here", async () => {
    const elsewhere: RecipeInstance = { ...slideInst, id: "r-s2", name: "Side show", screenId: "s2", flowIds: [] };
    const aimed: RecipeInstance = { ...slideInst, id: "r-s3", name: "Side strap", screenId: "s2", flowIds: ["fl3"] };
    const fl3: FlowInfo = { ...flow, id: "fl3", name: "Strap on main", origin: "r-s3" };
    await mount([eventInst, elsewhere, aimed], [flow, fl3]);
    expect(document.querySelector('[data-recipe="r-s2"]')).toBeNull();
    expect(panel("r-s3")).toBeTruthy();
  });

  it("same-named automations are told apart by their source, else their screen, else when they were added", async () => {
    const a: RecipeInstance = { ...slideInst, id: "r-a", name: "Sponsor slideshow", sourceId: "sp" };
    const b: RecipeInstance = { ...slideInst, id: "r-b", name: "Sponsor slideshow", sourceId: "sp2" };
    await mount([a, b], [], [summary("sp2", "Partners", "text")]);
    expect(txt(panel("r-a"))).toContain("— Sponsors (source)");
    expect(txt(panel("r-b"))).toContain("— Partners (source)");
    expect(screen.getByRole("button", { name: "Actions for Sponsor slideshow — Partners (source)" })).toBeTruthy();
  });

  it("warnings: counted per automation in the row, each reason once with the rules it affects", async () => {
    const msg = "The layer it takes was deleted (screen “Main”)";
    const w1: FlowInfo = { ...flow, warnings: [{ node: "a", message: msg }] };
    const w2: FlowInfo = { ...flow, id: "fl5", name: "Strap on NEXT", armed: false, warnings: [{ node: "a", message: msg }, { node: "t", message: "Source no longer exists" }] };
    await mount([{ ...eventInst, flowIds: ["fl1", "fl5"] }], [w1, w2]);
    expect(txt(panel("r-ev"))).toContain("2 rules · 1 armed · 2 warnings");
    await expand("r-ev");
    const list = within(panel("r-ev")).getByRole("list", { name: "Warnings of Event schedule" });
    const items = within(list).getAllByRole("listitem");
    expect(items.length).toBe(2);
    expect(items[0].textContent).toContain(msg);
    expect(items[0].textContent).toContain("Affects Strap on NOW, Strap on NEXT");
    expect(items[1].textContent).toContain("Affects Strap on NEXT");
    // Not repeated per rule.
    expect(within(panel("r-ev")).queryByText(/^\d+ warnings?$/)).toBeNull();
  });

  it("nothing here: says so and offers Add automation… and New rule", async () => {
    const ctl = await mount([], []);
    expect(screen.getByText("Nothing runs on its own here.")).toBeTruthy();
    expect(screen.getAllByRole("button", { name: "Add automation…" }).length).toBe(2);
    const show = vi.spyOn(ctl.flowEditor, "show").mockResolvedValue();
    await fireEvent.click(screen.getByRole("button", { name: "New rule" }));
    expect(show).toHaveBeenCalledWith({ mode: "new", kind: "rule" });
  });

  it("a rule's armed switch saves at once; a failure rolls back with a toast", async () => {
    await mount();
    await expand("r-ev");
    const sw = within(panel("r-ev")).getByRole("switch", { name: "Arm Strap on NOW" });
    await fireEvent.click(sw);
    await flush();
    expect(api.setFlowArmed).toHaveBeenLastCalledWith("fl1", false);
    expect(sw.getAttribute("aria-checked")).toBe("false");

    api.setFlowArmed.mockRejectedValueOnce(new Error("disk full"));
    await fireEvent.click(sw);
    await flush();
    expect(sw.getAttribute("aria-checked")).toBe("false");
    expect(toasts.list.at(-1)?.message).toBe("Couldn't arm Strap on NOW: disk full");
  });

  it("flow-state updates last fired; Edit opens the linear editor; the menus say Edit in Flows", async () => {
    const ctl = await mount();
    ctl.flows.connect();
    await flush();
    await expand("r-ev");
    events.handlers.get("flow-state")!({ id: "fl1", state: "running", node: "a", message: null, at: new Date(Date.now() - 125_000).toISOString() });
    await flush();
    expect(within(panel("r-ev")).getByText("2 min ago")).toBeTruthy();
    expect(within(panel("r-ev")).getByRole("status").textContent).toBe("Running");

    const show = vi.spyOn(ctl.flowEditor, "show").mockResolvedValue();
    await fireEvent.click(within(panel("r-ev")).getByRole("button", { name: "Actions for Strap on NOW" }));
    expect(within(panel("r-ev")).getByRole("menuitem", { name: "Edit in Flows" })).toBeTruthy();
    await fireEvent.click(within(panel("r-ev")).getByRole("menuitem", { name: "Edit" }));
    expect(show).toHaveBeenCalledWith({ mode: "edit", id: "fl1" });

    await fireEvent.click(within(panel("r-ev")).getByRole("button", { name: "Actions for Event schedule" }));
    const items = within(panel("r-ev")).getAllByRole("menuitem").map((m) => m.textContent?.trim());
    expect(items).toEqual(["Edit in Flows", "Remove…"]);
  });

  it("visible text uses operator words: no flow or recipe", async () => {
    await mount([eventInst, slideInst, { ...slideInst, id: "r-x", sourceId: "gone" }]);
    await expand("r-ev");
    expect(strip().textContent).not.toMatch(/flow|recipe/i);
  });
});

// ---- Needs attention ------------------------------------------------------------------------------

describe("automations that need attention", () => {
  const goneSource: RecipeInstance = { ...slideInst, id: "r-g1", name: "Sponsor slideshow", sourceId: "gone", flowIds: ["fg1"] };
  const goneLayer: RecipeInstance = { ...slideInst, id: "r-g2", name: "Sponsor slideshow", sourceId: "gone2", layerIds: ["deleted"], flowIds: ["fg2"] };
  const ruleOf = (id: string, origin: string): FlowInfo => ({ ...flow, id, name: "Pause the slideshow when it goes off air", origin });
  const banner = () => screen.getByRole("group", { name: /need attention|needs attention/ });

  it("broken automations are not listed: one banner with the count, a reason each, Fix… and Delete…", async () => {
    await mount([eventInst, goneSource, goneLayer], [flow, ruleOf("fg1", "r-g1"), ruleOf("fg2", "r-g2")]);
    expect(txt(banner())).toContain("2 automations need attention");
    expect(document.querySelector('[data-recipe="r-g1"]')).toBeNull();
    expect(document.querySelector('[data-flow="fg1"]')).toBeNull();
    const one = document.querySelector<HTMLElement>('[data-broken="r-g1"]')!;
    const two = document.querySelector<HTMLElement>('[data-broken="r-g2"]')!;
    expect(one.textContent).toContain("Its source was deleted.");
    expect(two.textContent).toContain("Its source and layer were deleted.");
    // Same names, same screen, no sources: told apart by when they were added.
    expect(one.textContent).toMatch(/— added \d+ \w{3} \d\d:\d\d/);
    expect(within(one).getByRole("button", { name: /^Fix Sponsor slideshow — added/ })).toBeTruthy();
  });

  it("Fix: pick another source of its kind; relinkRecipeSource; its rules reload", async () => {
    api.relinkRecipeSource.mockResolvedValue({ ...goneSource, sourceId: "sp2" });
    await mount([goneSource], [ruleOf("fg1", "r-g1")], [summary("sp2", "Partners", "text")]);
    await fireEvent.click(within(banner()).getByRole("button", { name: /^Fix/ }));
    const dlg = within(await screen.findByRole("dialog", { name: "Fix Sponsor slideshow" }));
    const select = dlg.getByLabelText("New source for Sponsor slideshow") as HTMLSelectElement;
    // Text sources only (a slideshow); the schedule is not offered.
    expect([...select.options].map((o) => o.textContent)).toEqual(["Sponsors", "Partners"]);
    await fireEvent.change(select, { target: { value: "sp2" } });
    api.listFlows.mockClear();
    await fireEvent.click(dlg.getByRole("button", { name: "Use this source" }));
    await flush();
    expect(api.relinkRecipeSource).toHaveBeenCalledWith("r-g1", "sp2");
    expect(api.listFlows).toHaveBeenCalled();
    expect(toasts.list.at(-1)?.message).toBe("Sponsor slideshow runs on Partners now.");
    // Fixed: a normal row now.
    expect(panel("r-g1")).toBeTruthy();
    expect(screen.queryByRole("group", { name: /attention/ })).toBeNull();
  });

  it("Fix: the server's refusal shows inline and the dialog stays", async () => {
    api.relinkRecipeSource.mockRejectedValue(new Error("This recipe needs a text source"));
    await mount([goneSource], [ruleOf("fg1", "r-g1")]);
    await fireEvent.click(within(banner()).getByRole("button", { name: /^Fix/ }));
    const dlg = within(await screen.findByRole("dialog", { name: "Fix Sponsor slideshow" }));
    await fireEvent.click(dlg.getByRole("button", { name: "Use this source" }));
    await flush();
    expect(dlg.getByRole("alert").textContent).toBe("This recipe needs a text source");
    expect(screen.getByRole("dialog", { name: "Fix Sponsor slideshow" })).toBeTruthy();
  });

  it("Fix with a layer gone explains why and offers Delete (the remove dialog, keep by default)", async () => {
    await mount([goneLayer], [ruleOf("fg2", "r-g2")]);
    await fireEvent.click(within(banner()).getByRole("button", { name: /^Fix/ }));
    const dlg = within(await screen.findByRole("dialog", { name: "Fix Sponsor slideshow" }));
    expect(dlg.getByText(/Its layer was deleted from “Main”/)).toBeTruthy();
    expect(dlg.queryByRole("button", { name: "Use this source" })).toBeNull();
    await fireEvent.click(dlg.getByRole("button", { name: "Delete…" }));
    const confirm = await screen.findByRole("button", { name: "Remove" });
    await fireEvent.click(confirm);
    await flush();
    expect(api.removeRecipe).toHaveBeenCalledWith("r-g2", false);
    expect(screen.queryByRole("group", { name: /attention/ })).toBeNull();
    expect(toasts.list.at(-1)?.message).toBe("Removed Sponsor slideshow.");
  });

  it("Delete… opens the remove dialog with its keep / delete-created choice", async () => {
    await mount([goneSource], [ruleOf("fg1", "r-g1")]);
    await fireEvent.click(within(banner()).getByRole("button", { name: /^Delete/ }));
    expect(await screen.findByRole("checkbox", { name: /Also delete what it created/ })).toBeTruthy();
    await fireEvent.click(screen.getByRole("checkbox", { name: /Also delete what it created/ }));
    await fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    await flush();
    expect(api.removeRecipe).toHaveBeenCalledWith("r-g1", true);
  });
});

describe("an automation's own rule", () => {
  it("New rule opens the editor aimed at the automation; Save creates the flow with it as origin", async () => {
    const ctl = await mount();
    await expand("r-sl");
    const show = vi.spyOn(ctl.flowEditor, "show").mockResolvedValue();
    await fireEvent.click(within(panel("r-sl")).getByRole("button", { name: "New rule for Sponsors" }));
    const target = { mode: "new", kind: "rule", origin: { id: "r-sl", screenId: "s1", layerId: "board", sourceId: "sp" } };
    expect(show).toHaveBeenCalledWith(target);

    api.createFlow.mockImplementation(async (f: NewFlow) => ({ ...flow, ...f, id: "fl9", origin: f.origin ?? null }));
    const onclose = vi.fn();
    render(LinearEditor, { props: { target: target as never, ctl, onclose } });
    await flush();
    const dlg = within(await screen.findByRole("dialog", { name: "New rule" }));
    // A slideshow's rule: when its slideshow wraps, then take its layer.
    expect((dlg.getByLabelText("Trigger") as HTMLSelectElement).value).toBe("trigger.slideshowWrap");
    expect((dlg.getByLabelText("Trigger source") as HTMLSelectElement).value).toBe("sp");
    await fireEvent.click(dlg.getByRole("button", { name: "Save" }));
    await flush();
    const req = api.createFlow.mock.calls.at(-1)![0] as NewFlow;
    expect(req).toMatchObject({ name: "Rule 1", armed: false, origin: "r-sl" });
    expect(toLinear(req)!.steps[0]).toMatchObject({ type: "action.take", config: { screenId: "s1", layerId: "board" } });
    await waitFor(() => expect(onclose).toHaveBeenCalled());
    // Listed under its automation (by origin).
    expect(within(panel("r-sl")).getByText("Rule 1")).toBeTruthy();
  });
});

// ---- Gallery ------------------------------------------------------------------------------------

describe("automation gallery", () => {
  async function openGallery() {
    await fireEvent.click(screen.getAllByRole("button", { name: "Add automation…" })[0]);
    return await screen.findByRole("option", { name: /Event schedule/ });
  }

  it("installs on an existing screen; its row appears (expanded) with a toast", async () => {
    const ctl = await mount([slideInst]);
    const select = vi.spyOn(ctl, "select");
    await fireEvent.click(await openGallery());
    expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Event schedule");
    expect((screen.getByLabelText("Screen") as HTMLSelectElement).value).toBe("s1");
    await fireEvent.change(screen.getByLabelText("Screen"), { target: { value: "s2" } });
    api.installRecipe.mockResolvedValue({ ...eventInst, screenId: "s2" });
    await fireEvent.click(screen.getByRole("button", { name: "Install" }));
    await flush();
    expect(api.installRecipe).toHaveBeenCalledWith({
      recipeId: "event-schedule",
      name: "Event schedule",
      screen: { kind: "existing", screenId: "s2" },
    });
    expect(select).toHaveBeenCalledWith("s2");
    expect(toasts.list.at(-1)?.message).toBe("Event schedule is ready on Side.");
  });

  it("installs on a new screen with a resolution preset; a failure stays open", async () => {
    await mount([]);
    await fireEvent.click(await openGallery());
    await fireEvent.input(screen.getByLabelText("Name"), { target: { value: "Day one" } });
    await fireEvent.click(screen.getByRole("radio", { name: "New screen" }));
    await fireEvent.input(screen.getByLabelText("Screen name"), { target: { value: "Schedule wall" } });
    await fireEvent.change(screen.getByLabelText("Resolution"), { target: { value: "720p" } });
    api.installRecipe.mockRejectedValueOnce(new Error("no space"));
    await fireEvent.click(screen.getByRole("button", { name: "Install" }));
    await flush();
    expect(api.installRecipe).toHaveBeenCalledWith({
      recipeId: "event-schedule",
      name: "Day one",
      screen: { kind: "new", name: "Schedule wall", width: 1280, height: 720 },
    });
    expect(screen.getByRole("alert").textContent).toBe("no space");
    expect(screen.getByRole("button", { name: "Install" })).toBeTruthy();
  });

  it("Install is disabled until a recipe is chosen", async () => {
    await mount([]);
    await openGallery();
    // Operator words in Control.
    expect(screen.getByRole("dialog", { name: "Add automation" })).toBeTruthy();
    expect((screen.getByRole("button", { name: "Install" }) as HTMLButtonElement).disabled).toBe(true);
  });
});

// ---- Remove -------------------------------------------------------------------------------------

describe("remove an automation", () => {
  async function openRemove(id: string, name: string) {
    await fireEvent.click(within(panel(id)).getByRole("button", { name: `Actions for ${name}` }));
    await fireEvent.click(within(panel(id)).getByRole("menuitem", { name: "Remove…" }));
    return await screen.findByRole("button", { name: "Remove" });
  }

  it("keeps what it created by default", async () => {
    await mount();
    await fireEvent.click(await openRemove("r-sl", "Sponsors"));
    await flush();
    expect(api.removeRecipe).toHaveBeenCalledWith("r-sl", false);
    expect(document.querySelector('[data-recipe="r-sl"]')).toBeNull();
    expect(toasts.list.at(-1)?.message).toBe("Removed Sponsors.");
  });

  it("can also delete what it created", async () => {
    await mount();
    const confirm = await openRemove("r-ev", "Event schedule");
    const box = screen.getByRole("checkbox", { name: /Also delete what it created \(its source, 2 templates, 2 layers\)/ });
    await fireEvent.click(box);
    expect(screen.getByText(/can't be undone/)).toBeTruthy();
    await fireEvent.click(confirm);
    await flush();
    expect(api.removeRecipe).toHaveBeenCalledWith("r-ev", true);
    expect(api.listSources).toHaveBeenCalled();
    expect(document.querySelector('[data-recipe="r-ev"]')).toBeNull();
  });
});

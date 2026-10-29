// The /flows list: grouping and summaries, and the page (arm, rename, duplicate, delete, new flow,
// the empty state, the palette's ?new). Also: @xyflow/svelte is only ever imported by the editor's
// lazy canvas folder.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { FlowInfo, FlowNode, LiveScreenView, NewFlow, RecipeInstance, UiState } from "$lib/types";

// Lazily loaded components these tests open: loaded once here, so the tests wait on them, not on
// Vite compiling them on first use (CLAUDE.md › Testing policy).
beforeAll(async () => {
  await Promise.all([
    import("$lib/components/flows/list/summaries.svelte"),
    import("$lib/components/flows/list/NewFlowDialog.svelte"),
    import("$lib/components/ConfirmDialog.svelte"),
    import("$lib/components/recipes/RecipeGallery.svelte"),
    import("$lib/components/flows/list/FirstFlow.svelte"),
  ]);
});

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Set<Handler>>() }));
const nav = vi.hoisted(() => ({ goto: vi.fn(), url: new URL("http://localhost/flows") }));
const api = vi.hoisted(() => ({
  listFlows: vi.fn(),
  getFlow: vi.fn(),
  createFlow: vi.fn(),
  renameFlow: vi.fn(),
  duplicateFlow: vi.fn(),
  deleteFlow: vi.fn(),
  setFlowArmed: vi.fn(),
  listSources: vi.fn(),
  listScreens: vi.fn(),
  listRecipeInstances: vi.fn(),
  listRecipes: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  on: (event: string, h: Handler) => {
    const set = events.handlers.get(event) ?? new Set();
    set.add(h);
    events.handlers.set(event, set);
    return () => set.delete(h);
  },
  toAppError: (e: unknown) => (e && typeof e === "object" && "message" in e ? e : { code: "internal", message: String(e) }),
}));
vi.mock("$lib/api/flows", () => api);
vi.mock("$lib/api/sources", () => api);
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/recipes", () => api);
vi.mock("$app/navigation", () => ({ goto: nav.goto }));
vi.mock("$app/state", () => ({
  page: {
    get url() {
      return nav.url;
    },
  },
}));

import { toasts } from "$lib/stores/toasts.svelte";
import { uiState } from "$lib/stores/uiState.svelte";
import FlowsPage from "../../../../routes/(app)/flows/+page.svelte";
import { groupFlows } from "./groups";
import { triggerSummary } from "./summary";

const node = (id: string, type: string, config: object, x = 0): FlowNode =>
  ({ id, type, config, position: { x, y: 0 } }) as FlowNode;

function flow(id: string, name: string, patch: Partial<FlowInfo> = {}): FlowInfo {
  return {
    id,
    name,
    armed: true,
    retrigger: "restart",
    origin: null,
    nodes: [
      node(`${id}a`, "trigger.cue", { label: null }),
      node(`${id}b`, "action.clearScreen", { screenId: "s1" }, 240),
    ],
    edges: [{ id: `${id}w`, from: { node: `${id}a`, port: "fired" }, to: { node: `${id}b`, port: "in" }, kind: "event" }],
    draft: null,
    lastFiredAt: null,
    updatedAt: "2026-10-04T10:00:00Z",
    warnings: [],
    running: false,
    readOnly: false,
    ...patch,
  };
}

const branchy = flow("f3", "Branchy", {
  nodes: [
    node("t", "trigger.layer", { screenId: "s1", layerId: "l1", onAir: true }),
    node("b", "logic.branch", {}),
    node("x", "action.clearAll", {}),
  ],
  edges: [],
  draft: { nodes: [], edges: [], updatedAt: "x" },
  warnings: [{ node: "b", message: "Condition is not wired" }],
});
const view: LiveScreenView = {
  screen: { id: "s1", name: "Main", width: 1920, height: 1080, background: "transparent" },
  layers: [{ id: "l1", name: "Guest", onAir: false, pending: false, fields: {}, fieldDefs: [] }],
};
const recipe = { id: "r1", name: "Main stage" } as RecipeInstance;

let list: FlowInfo[];

beforeEach(() => {
  events.handlers.clear();
  toasts.list = [];
  nav.url = new URL("http://localhost/flows");
  nav.goto.mockReset().mockResolvedValue(undefined);
  for (const f of Object.values(api)) f.mockReset();
  list = [
    flow("f2", "Opener"),
    flow("f1", "Break", { armed: false, lastFiredAt: new Date(Date.now() - 120_000).toISOString() }),
    branchy,
    flow("r1f", "When an item starts", { origin: "r1" }),
    flow("f9", "Future", { readOnly: true }),
  ];
  api.listFlows.mockImplementation(async () => structuredClone(list));
  api.listSources.mockResolvedValue([]);
  api.listScreens.mockResolvedValue([view]);
  api.listRecipeInstances.mockResolvedValue([recipe]);
  uiState.set({ sidebarPinned: false, control: { hotkeys: { "flow/f1": "F2" } } } as unknown as UiState);
});

afterEach(cleanup);

// "Add recipe" opens the recipe gallery, a lazy chunk. Load it once here, so the test never waits on
// it being transformed (over findBy's 1 s on a busy machine, in a full run).
beforeAll(async () => {
  await (await import("$lib/components/recipes/lazy")).loadGallery();
}, 30_000);

describe("groups and summaries", () => {
  const k = { screens: new Map([["s1", view]]), sources: new Map(), flows: new Map() };

  it("own flows first (by name), then one group per recipe; a removed recipe says so", () => {
    const g = groupFlows(list, new Map([["r1", "Main stage"]]));
    expect(g.map((x) => [x.name, x.recipe, x.flows.map((f) => f.name)])).toEqual([
      ["Project", false, ["Branchy", "Break", "Future", "Opener"]],
      ["Main stage", true, ["When an item starts"]],
    ]);
    expect(groupFlows([flow("z", "Z", { origin: "gone" })], new Map())[0].name).toBe("Removed recipe");
  });

  it("linear flows read as When … then …; others as trigger types and size", () => {
    expect(triggerSummary(list[0], k)).toBe("When cued then clear Main");
    expect(triggerSummary(branchy, k)).toBe("Layer on/off air · 3 nodes");
    expect(triggerSummary({ nodes: [node("x", "action.clearAll", {})], edges: [] }, k)).toBe("No trigger · 1 node");
  });
});

describe("/flows page", () => {
  const row = (id: string) => document.querySelector<HTMLElement>(`[data-flow="${id}"]`)!;

  it("groups, rows, notes; names open the editor", async () => {
    render(FlowsPage);
    const own = await screen.findByRole("list", { name: "Project flows" });
    expect(within(own).getAllByRole("listitem")).toHaveLength(4);
    expect(screen.getByRole("list", { name: "Main stage flows" })).toBeTruthy();
    expect(row("f3").textContent).toContain("Not deployed");
    expect(row("f3").textContent).toContain("1 warning");
    expect(row("f1").querySelector(".fired")?.textContent?.trim()).toBe("2 min ago");
    expect(row("f9").textContent).toContain("Read-only");
    // Summaries arrive with their lazy chunk and name the screens once listed.
    await waitFor(() => expect(row("f2").querySelector(".sum")?.textContent).toBe("When cued then clear Main"));
    expect(row("f3").querySelector(".sum")?.textContent).toBe("Layer on/off air · 3 nodes");
    expect(within(row("f2")).getByRole("link", { name: "Opener" }).getAttribute("href")).toBe("/flows/f2");
    expect((within(row("f9")).getByRole("button", { name: "Delete Future…" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("arm is optimistic and rolls back on failure", async () => {
    api.setFlowArmed.mockRejectedValue({ code: "internal", message: "disk full" });
    render(FlowsPage);
    const sw = await screen.findByRole("switch", { name: "Arm Opener" });
    await fireEvent.click(sw);
    await waitFor(() => expect(toasts.list.at(-1)?.message).toContain("disk full"));
    expect(sw.getAttribute("aria-checked")).toBe("true");
  });

  it("rename in place: Enter saves, Escape cancels", async () => {
    api.renameFlow.mockImplementation(async (id: string, name: string) => ({ ...list.find((f) => f.id === id)!, name }));
    render(FlowsPage);
    await fireEvent.click(await screen.findByRole("button", { name: "Rename Opener" }));
    const input = await screen.findByRole("textbox", { name: "Name of Opener" });
    await fireEvent.input(input, { target: { value: "Show opener" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(api.renameFlow).toHaveBeenCalledWith("f2", "Show opener"));
    expect(await screen.findByRole("link", { name: "Show opener" })).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Rename Break" }));
    const again = await screen.findByRole("textbox", { name: "Name of Break" });
    await fireEvent.input(again, { target: { value: "Nope" } });
    await fireEvent.keyDown(again, { key: "Escape" });
    expect(api.renameFlow).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("link", { name: "Break" })).toBeTruthy();
  });

  it("duplicate adds the copy; delete asks first and drops the hotkey", async () => {
    api.duplicateFlow.mockResolvedValue(flow("f2c", "Opener copy", { armed: false }));
    api.deleteFlow.mockResolvedValue(undefined);
    render(FlowsPage);
    await fireEvent.click(await screen.findByRole("button", { name: "Duplicate Opener" }));
    expect(await screen.findByRole("link", { name: "Opener copy" })).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Delete Break…" }));
    const dialog = await screen.findByRole("dialog", { name: "Delete “Break”?" });
    expect(api.deleteFlow).not.toHaveBeenCalled();
    await fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(api.deleteFlow).toHaveBeenCalledWith("f1"));
    await waitFor(() => expect(screen.queryByRole("link", { name: "Break" })).toBeNull());
    expect(uiState.value?.control.hotkeys).toEqual({});
  });

  it("New flow: a name, then an armed flow with one Cue trigger, opened in the editor", async () => {
    api.createFlow.mockImplementation(async (f: NewFlow) => flow("new1", f.name, { nodes: f.nodes, edges: f.edges }));
    render(FlowsPage);
    await fireEvent.click(await screen.findByRole("button", { name: "New flow" }));
    const dialog = await screen.findByRole("dialog", { name: "New flow" });
    const name = within(dialog).getByRole("textbox", { name: "Name" }) as HTMLInputElement;
    expect(name.value).toBe("Flow 1");
    await fireEvent.input(name, { target: { value: "Lights" } });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Create" }));
    await waitFor(() => expect(nav.goto).toHaveBeenCalledWith("/flows/new1"));
    const sent = api.createFlow.mock.calls[0][0] as NewFlow;
    expect(sent).toMatchObject({ name: "Lights", armed: true, retrigger: "restart", edges: [] });
    expect(sent.nodes.map((n) => n.type)).toEqual(["trigger.cue"]);
  });

  it("the palette's New flow (?new) opens the dialog", async () => {
    nav.url = new URL("http://localhost/flows?new");
    render(FlowsPage);
    expect(await screen.findByRole("dialog", { name: "New flow" })).toBeTruthy();
    expect(nav.goto).toHaveBeenCalledWith("/flows", expect.objectContaining({ replaceState: true }));
  });

  it("empty: Create your first flow and Add recipe", async () => {
    list = [];
    api.listRecipes.mockResolvedValue([]);
    render(FlowsPage);
    expect(await screen.findByRole("button", { name: "Create your first flow" })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Add recipe" }));
    expect(await screen.findByRole("dialog", { name: "Add recipe" })).toBeTruthy();
  });
});

describe("@xyflow/svelte stays in the editor's lazy canvas", () => {
  it("is imported only under flows/graph/canvas/", () => {
    const root = join(process.cwd(), "src");
    const offenders: string[] = [];
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const p = join(dir, name);
        if (statSync(p).isDirectory()) walk(p);
        else if (/\.(svelte|ts)$/.test(p) && !p.endsWith(".test.ts")) {
          if (/from\s+["']@xyflow\//.test(readFileSync(p, "utf8")) && !/flows[\\/]graph[\\/]canvas[\\/]/.test(p)) offenders.push(p);
        }
      }
    };
    walk(root);
    expect(offenders).toEqual([]);
    // …and the canvas folder is reached only through load.ts's dynamic import().
    const load = readFileSync(join(root, "lib/components/flows/graph/load.ts"), "utf8");
    expect(load).toContain('import("./canvas/Canvas.svelte")');
  });
});

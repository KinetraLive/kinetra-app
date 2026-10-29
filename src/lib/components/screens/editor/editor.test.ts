import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  FilesChanged,
  LiveLayerView,
  ScreenDoc as AnyScreenDoc,
  TemplateScreenLayer as ScreenLayer,
  ScreenState,
  ServerStatus,
  TemplateDoc,
  SourceSummary,
  TemplateSummary,
} from "$lib/types";

// ---- Mocks --------------------------------------------------------------------------------------

// Lazily loaded components these tests open: loaded once here, so the tests wait on them, not on
// Vite compiling them on first use (CLAUDE.md › Testing policy).
beforeAll(async () => {
  await Promise.all([
    import("$lib/components/ConfirmDialog.svelte"),
    import("$lib/components/templates/editor/DiskBanner.svelte"),
  ]);
});

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Set<(p: unknown) => void>>() }));

const api = vi.hoisted(() => ({
  getScreen: vi.fn(),
  saveScreen: vi.fn(),
  setLayerRect: vi.fn(),
  setLayerVariant: vi.fn(),
  renameScreen: vi.fn(),
  duplicateScreen: vi.fn(),
  deleteScreen: vi.fn(),
  liveShow: vi.fn(),
  liveHide: vi.fn(),
  serverStatus: vi.fn(),
  listTemplates: vi.fn(),
  getTemplate: vi.fn(),
  listSources: vi.fn(),
  setTimer: vi.fn(),
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  on: vi.fn(),
  getSource: vi.fn(),
  saveSource: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e
      ? e
      : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
vi.mock("$lib/api/screens", () => api);
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/templates", () => api);
vi.mock("$lib/api/sources", () => api);
vi.mock("$lib/api/automation", () => api);
// The spreadsheet editor (its own chunk): a stand-in that records its props.
vi.mock("$lib/components/sheet/SheetEditor.svelte", async () => ({
  default: (await import("../../fields/SheetStub.test.svelte")).default,
}));

const gallery = vi.hoisted(() => ({ show: vi.fn() }));
vi.mock("$lib/components/templates/gallery/launcher.svelte", () => ({
  GalleryLauncher: class {
    component = null;
    open = false;
    show = gallery.show;
    close = () => {};
  },
}));

const nav = vi.hoisted(() => ({ goto: vi.fn() }));
vi.mock("$app/navigation", () => nav);
vi.mock("$app/state", () => ({
  page: { params: { id: "s1" }, url: new URL("http://localhost/screens/s1") },
}));

/** Observers that report a visible 960×540 box at once (scale 0.5 for a 1920×1080 screen). */
class FakeObserver {
  constructor(private readonly cb: (entries: unknown[]) => void) {}
  observe() {
    this.cb([{ isIntersecting: true, contentRect: { width: 960, height: 540 } }]);
  }
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
}
vi.stubGlobal("IntersectionObserver", FakeObserver);
vi.stubGlobal("ResizeObserver", FakeObserver);

class FakePointerEvent extends MouseEvent {
  pointerId: number;
  constructor(type: string, init: PointerEventInit = {}) {
    super(type, init);
    this.pointerId = init.pointerId ?? 1;
  }
}
vi.stubGlobal("PointerEvent", FakePointerEvent);

import ScreenEditor from "./ScreenEditor.svelte";
import RoutePage from "../../../../routes/(app)/screens/[id]/+page.svelte";
import { server } from "$lib/stores/server.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { registry, runCommand, titleOf } from "$lib/commands/registry.svelte";
/** Test docs hold template layers only. */
type ScreenDoc = Omit<AnyScreenDoc, "layers"> & { layers: ScreenLayer[] };

// The editor's panels are lazy chunks: load them once up front, so a cold first import under a
// busy parallel run doesn't eat the first test's time.
beforeAll(async () => {
  await Promise.all([import("./LayersPanel.svelte"), import("./Toolbar.svelte"), import("./Inspector.svelte")]);
});

// ---- Fixtures -----------------------------------------------------------------------------------

const running: ServerStatus = { state: "running", port: 7770, url: "http://127.0.0.1:7770", lanUrl: null };

function makeTemplate(id: string, name: string): TemplateDoc {
  return {
    id,
    name,
    description: "",
    preset: null,
    fields: [
      { key: "name", label: "Name", type: "text", default: "Ada" },
      { key: "title", label: "Title", type: "multiline", default: "" },
    ],
    variables: [
      { key: "accent", label: "Accent", type: "color", default: "#22b8cf" },
      { key: "radius", label: "Corner radius", type: "range", min: 0, max: 24, step: 1, unit: "px", default: "4" },
    ],
    autoOutMs: null,
    updatedAt: "2026-10-03T20:00:00Z",
    defaultVariant: "default",
    variants: [{ id: "default", name: "Default", vars: {}, html: "<div></div>", css: "" }],
  };
}

const summaries: TemplateSummary[] = [
  { id: "t1", name: "Classic bar", description: "", preset: null, fieldCount: 2, variableCount: 2, variantCount: 1, updatedAt: "", presetUpdate: null },
  { id: "t2", name: "News strap", description: "", preset: null, fieldCount: 2, variableCount: 2, variantCount: 1, updatedAt: "", presetUpdate: null },
];

const sourceList: SourceSummary[] = [
  {
    id: "g1",
    name: "Guests",
    kind: "text",
    rowCount: 2,
    index: 1,
    current: { name: "Grace Hopper", title: "Rear Admiral" },
    columns: [
      { key: "name", label: "Name" },
      { key: "title", label: "Title" },
    ],
    usedBy: [],
    auto: null,
    nextAt: null,
    nowId: null,
    nextId: null,
    updatedAt: "",
  },
];

const layer = (id: string, over: Partial<ScreenLayer> = {}): ScreenLayer => ({
  id,
  name: `Layer ${id}`,
  kind: "template",
  templateId: "t1",
  rect: { x: 0, y: 0, w: 1920, h: 1080 },
  fields: { name: "Ada", title: "" },
  vars: {},
  ...over,
});

function makeDoc(): ScreenDoc {
  return {
    id: "s1",
    name: "Main program",
    width: 1920,
    height: 1080,
    background: "transparent",
    // Bottom first: a, b, c (c is in front).
    layers: [
      layer("a"),
      layer("b", { rect: { x: 100, y: 100, w: 400, h: 200 } }),
      layer("c", { templateId: "gone" }),
    ],
    updatedAt: "2026-10-03T20:00:00Z",
  };
}

const liveView = (id: string, onAir: boolean, pending = false): LiveLayerView => ({
  id,
  name: `Layer ${id}`,
  onAir,
  pending,
  fields: {},
  fieldDefs: [],
});

function stateFor(doc: ScreenDoc): ScreenState {
  return {
    doc,
    live: {
      screen: { id: doc.id, name: doc.name, width: doc.width, height: doc.height, background: doc.background },
      layers: doc.layers
        .filter((l) => l.templateId !== "gone")
        .map((l) => (l.id === "a" ? liveView("a", true, true) : liveView(l.id, false))),
    },
    warnings: doc.layers
      .filter((l) => l.templateId === "gone")
      .map((l) => ({ layerId: l.id, code: "templateMissing" as const, message: "Template not found" })),
  };
}

function emit(event: string, payload: unknown) {
  for (const h of events.handlers.get(event) ?? []) h(payload);
}

async function advance(ms: number) {
  await vi.advanceTimersByTimeAsync(ms);
  await tick();
}

async function mount() {
  const r = render(ScreenEditor, { id: "s1" });
  await vi.waitFor(() => expect(r.container.querySelector("li[data-layer]")).toBeTruthy());
  await advance(0);
  return r;
}

const row = (c: HTMLElement, id: string) => c.querySelector<HTMLElement>(`li[data-layer="${id}"]`)!;
const main = (c: HTMLElement, id: string) => row(c, id).querySelector<HTMLElement>(".main")!;
const box = (c: HTMLElement, id: string) => c.querySelector<HTMLElement>(`.box[data-layer="${id}"]`)!;
const panelOrder = (c: HTMLElement) =>
  [...c.querySelectorAll<HTMLElement>("li[data-layer]")].map((el) => el.dataset.layer);
const lastSaved = (): ScreenDoc => api.saveScreen.mock.calls.at(-1)?.[0];

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => setTimeout(() => cb(0), 16) as unknown as number);
  vi.stubGlobal("cancelAnimationFrame", (id: number) => clearTimeout(id));
  events.handlers.clear();
  toasts.clear();
  server.status = null;
  api.on.mockImplementation((event: string, h: Handler) => {
    let set = events.handlers.get(event);
    if (!set) events.handlers.set(event, (set = new Set()));
    set.add(h);
    return () => set.delete(h);
  });
  api.getScreen.mockImplementation(async () => stateFor(makeDoc()));
  api.saveScreen.mockImplementation(async (doc: ScreenDoc) =>
    stateFor({ ...doc, updatedAt: "2026-10-03T21:00:00Z" }),
  );
  api.setLayerRect.mockImplementation(async (_s: string, id: string) => liveView(id, false));
  api.liveShow.mockImplementation(async (_s: string, id: string) => liveView(id, true));
  api.liveHide.mockImplementation(async (_s: string, id: string) => liveView(id, false));
  api.serverStatus.mockResolvedValue(running);
  api.listTemplates.mockResolvedValue(summaries);
  api.listSources.mockResolvedValue(sourceList);
  api.getTemplate.mockImplementation(async (id: string) => {
    const s = summaries.find((t) => t.id === id);
    if (!s) throw { code: "notFound", message: "Template not found" };
    return makeTemplate(id, s.name);
  });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

// ---- Load ---------------------------------------------------------------------------------------

describe("load", () => {
  it("lists layers top = front with tally, template name and markers", async () => {
    const { container } = await mount();
    expect(api.getScreen).toHaveBeenCalledWith("s1");
    expect(panelOrder(container)).toEqual(["c", "b", "a"]);
    const a = row(container, "a");
    expect(within(a).getByRole("img", { name: "On air" })).toBeTruthy();
    expect(within(a).getByText("Changes on next take")).toBeTruthy();
    expect(within(a).getByText("Classic bar")).toBeTruthy();
    expect(within(row(container, "b")).getByRole("img", { name: "Off air" })).toBeTruthy();
    expect(within(row(container, "b")).queryByText("Changes on next take")).toBeNull();
    const c = row(container, "c");
    expect(within(c).getByText("Template missing")).toBeTruthy();
    expect((within(c).getByRole("button", { name: "Play in Layer c" }) as HTMLButtonElement).disabled).toBe(true);
    // Each used template is loaded once; the missing one fails softly.
    expect(api.getTemplate.mock.calls.map((c) => c[0]).sort()).toEqual(["gone", "t1"]);
  });

  it("has exactly one iframe: the canvas preview of the real screen URL", async () => {
    const { container } = await mount();
    await vi.waitFor(() => expect(container.querySelectorAll("iframe")).toHaveLength(1));
    expect(container.querySelector("iframe")?.getAttribute("src")).toBe("http://127.0.0.1:7770/screen/s1?mute=1");
    expect(screen.getByText("http://127.0.0.1:7770/screen/s1")).toBeTruthy();
    expect(screen.getByText("1920×1080")).toBeTruthy();
  });

  it("the route page reads the id from the URL", async () => {
    render(RoutePage);
    await vi.waitFor(() => expect(api.getScreen).toHaveBeenCalledWith("s1"));
  });

  it("shows the error when the screen can't be loaded", async () => {
    api.getScreen.mockRejectedValueOnce({ code: "notFound", message: "Screen not found" });
    render(ScreenEditor, { id: "s1" });
    await vi.waitFor(() => expect(screen.getByText(/Couldn't open this screen: Screen not found/)).toBeTruthy());
  });

  it("no input has a placeholder", async () => {
    const { container } = await mount();
    await fireEvent.click(main(container, "b"));
    expect(document.querySelectorAll("[placeholder]")).toHaveLength(0);
  });
});

// ---- Layers ---------------------------------------------------------------------------------------

describe("layers panel", () => {
  it("Alt+ArrowUp / Alt+ArrowDown reorder the doc (z-order) and save once", async () => {
    const { container } = await mount();
    const aMain = main(container, "a");
    await fireEvent.keyDown(aMain, { key: "ArrowUp", altKey: true });
    expect(panelOrder(container)).toEqual(["c", "a", "b"]);
    await fireEvent.keyDown(aMain, { key: "ArrowUp", altKey: true });
    expect(panelOrder(container)).toEqual(["a", "c", "b"]);
    // At the top: no change.
    await fireEvent.keyDown(aMain, { key: "ArrowUp", altKey: true });
    await fireEvent.keyDown(main(container, "b"), {
      key: "ArrowUp",
      altKey: true,
    });
    expect(panelOrder(container)).toEqual(["a", "b", "c"]);
    await advance(499);
    expect(api.saveScreen).not.toHaveBeenCalled();
    await advance(1);
    expect(api.saveScreen).toHaveBeenCalledTimes(1);
    // Doc order is bottom first.
    expect(lastSaved().layers.map((l) => l.id)).toEqual(["c", "b", "a"]);
    await fireEvent.keyDown(aMain, { key: "ArrowDown", altKey: true });
    await advance(500);
    expect(lastSaved().layers.map((l) => l.id)).toEqual(["c", "a", "b"]);
  });

  it("adds a layer from the template picker: full screen, template defaults, in front", async () => {
    const { container } = await mount();
    // The Add layer menu is its own chunk: wait for it before using it.
    await vi.dynamicImportSettled();
    await tick();
    await fireEvent.click(screen.getByRole("button", { name: "Add layer" }));
    await tick();
    const menu = screen.getByRole("menu");
    expect(within(menu).getAllByRole("menuitem").map((m) => m.textContent?.replace(/\s+/g, " ").trim())).toEqual([
      "Classic bar 2 fields",
      "News strap 2 fields",
      "Media…",
      "New template…",
    ]);
    await fireEvent.click(within(menu).getByRole("menuitem", { name: /News strap/ }));
    await advance(0);
    expect(screen.queryByRole("menu")).toBeNull();
    expect(panelOrder(container)).toHaveLength(4);
    await advance(500);
    const added = lastSaved().layers.at(-1)!;
    expect(added).toMatchObject({
      name: "News strap",
      templateId: "t2",
      kind: "template",
      rect: { x: 0, y: 0, w: 1920, h: 1080 },
      fields: { name: "Ada", title: "" },
      vars: {},
    });
    expect(added.id).toMatch(/^[A-Za-z0-9_-]{10}$/);
    // The new layer is selected: the inspector shows it.
    expect((screen.getByLabelText("Name", { selector: "input[maxlength]" }) as HTMLInputElement).value).toBe(
      "News strap",
    );
  });

  it("New template… opens the gallery", async () => {
    await mount();
    // The Add layer menu is its own chunk: wait for it before using it.
    await vi.dynamicImportSettled();
    await tick();
    await fireEvent.click(screen.getByRole("button", { name: "Add layer" }));
    await tick();
    await fireEvent.click(screen.getByRole("menuitem", { name: "New template…" }));
    expect(gallery.show).toHaveBeenCalledTimes(1);
  });

  it("deletes a layer with its button or the Delete key", async () => {
    const { container } = await mount();
    await fireEvent.click(within(row(container, "b")).getByRole("button", { name: "Delete layer" }));
    expect(panelOrder(container)).toEqual(["c", "a"]);
    await fireEvent.keyDown(main(container, "c"), { key: "Delete" });
    expect(panelOrder(container)).toEqual(["a"]);
    await advance(500);
    expect(api.saveScreen).toHaveBeenCalledTimes(1);
    expect(lastSaved().layers.map((l) => l.id)).toEqual(["a"]);
  });

  it("Play in / Play out call liveShow / liveHide and update the tally", async () => {
    const { container } = await mount();
    await fireEvent.click(within(row(container, "b")).getByRole("button", { name: "Play in Layer b" }));
    await advance(0);
    expect(api.liveShow).toHaveBeenCalledWith("s1", "b");
    expect(within(row(container, "b")).getByRole("img", { name: "On air" })).toBeTruthy();
    await fireEvent.click(within(row(container, "a")).getByRole("button", { name: "Play out Layer a" }));
    await advance(0);
    expect(api.liveHide).toHaveBeenCalledWith("s1", "a");
    expect(within(row(container, "a")).getByRole("img", { name: "Off air" })).toBeTruthy();
  });

  it("Play in saves pending edits first", async () => {
    const { container } = await mount();
    await fireEvent.click(within(row(container, "b")).getByRole("button", { name: "Delete layer" }));
    await fireEvent.click(within(row(container, "c")).getByRole("button", { name: "Delete layer" }));
    await fireEvent.click(within(row(container, "a")).getByRole("button", { name: "Play out Layer a" }));
    await advance(0);
    expect(api.saveScreen).toHaveBeenCalledTimes(1);
    expect(api.saveScreen.mock.invocationCallOrder[0]).toBeLessThan(api.liveHide.mock.invocationCallOrder[0]);
  });

  it("selecting a row selects the box on the canvas, and back", async () => {
    const { container } = await mount();
    await fireEvent.click(main(container, "b"));
    expect(box(container, "b").hasAttribute("data-selected")).toBe(true);
    expect(row(container, "b").hasAttribute("data-selected")).toBe(true);
    expect(box(container, "b").querySelectorAll(".handle")).toHaveLength(8);
    await fireEvent.focus(box(container, "a"));
    expect(row(container, "a").hasAttribute("data-selected")).toBe(true);
    expect(box(container, "b").hasAttribute("data-selected")).toBe(false);
  });
});

// ---- Canvas -----------------------------------------------------------------------------------------

describe("canvas", () => {
  it("boxes sit at their rect scaled to the stage", async () => {
    const { container } = await mount();
    const b = box(container, "b");
    expect(b.style.transform).toBe("translate(50px, 50px)");
    expect(b.style.width).toBe("200px");
    expect(b.style.height).toBe("100px");
  });

  it("a drag moves only the box, sends <= 20 setLayerRect/s and saves once 500 ms after the drop", async () => {
    const { container } = await mount();
    const b = box(container, "b");
    await fireEvent.pointerDown(b, { clientX: 100, clientY: 100, button: 0, pointerId: 1 });
    // One second of pointer moves every 4 ms, 1 canvas px each (= 2 screen px), Alt: no snapping.
    for (let i = 1; i <= 250; i++) {
      await fireEvent.pointerMove(b, { clientX: 100 + i, clientY: 100, pointerId: 1, altKey: true });
      await vi.advanceTimersByTimeAsync(4);
    }
    await vi.advanceTimersByTimeAsync(16);
    const during = api.setLayerRect.mock.calls.length;
    expect(during).toBeGreaterThan(5);
    expect(during).toBeLessThanOrEqual(21);
    expect(api.saveScreen).not.toHaveBeenCalled();
    // Only the overlay box moved, by transform; its size is unchanged.
    expect(b.style.transform).toBe("translate(300px, 50px)");
    expect(b.style.width).toBe("200px");

    await fireEvent.pointerUp(b, { clientX: 350, clientY: 100, pointerId: 1, altKey: true });
    await advance(0);
    const last = api.setLayerRect.mock.calls.at(-1)!;
    expect(last).toEqual(["s1", "b", { x: 600, y: 100, w: 400, h: 200 }]);
    await advance(499);
    expect(api.saveScreen).not.toHaveBeenCalled();
    await advance(1);
    expect(api.saveScreen).toHaveBeenCalledTimes(1);
    expect(lastSaved().layers.find((l) => l.id === "b")?.rect).toEqual({ x: 600, y: 100, w: 400, h: 200 });
    await advance(2000);
    expect(api.saveScreen).toHaveBeenCalledTimes(1);
  });

  it("snaps to the screen centre without Alt and shows a guide", async () => {
    const { container } = await mount();
    const b = box(container, "b");
    // Box centre x = 300 screen px; move by 328 canvas px... aim the centre near 960 (dx ~ 660 screen = 330 canvas).
    await fireEvent.pointerDown(b, { clientX: 100, clientY: 100, button: 0 });
    await fireEvent.pointerMove(b, { clientX: 100 + 328, clientY: 100 });
    await vi.advanceTimersByTimeAsync(16);
    expect(container.querySelector(".guide.x")).toBeTruthy();
    await fireEvent.pointerUp(b, { clientX: 428, clientY: 100 });
    await advance(500);
    const r = lastSaved().layers.find((l) => l.id === "b")!.rect;
    expect(r.x + r.w / 2).toBe(960);
    expect(container.querySelector(".guide")).toBeNull();
  });

  /** Layer b explicitly Reflow: the free resize of before. */
  const reflowB = () =>
    api.getScreen.mockImplementation(async () => {
      const d = makeDoc();
      d.layers[1] = { ...d.layers[1], sizing: "reflow" };
      return stateFor(d);
    });

  it("resizing scales: the graphic's aspect is kept and the layer becomes Scale to fit; Shift stretches", async () => {
    const { container } = await mount();
    await fireEvent.click(main(container, "b"));
    const se = () => box(container, "b").querySelector<HTMLElement>('[data-handle="se"]')!;
    await fireEvent.pointerDown(se(), { clientX: 300, clientY: 150, button: 0 });
    await fireEvent.pointerMove(se(), { clientX: 400, clientY: 160, ctrlKey: true });
    await advance(16);
    // The switch to fit is saved at the first frame, before the drag's live rects.
    expect(api.saveScreen).toHaveBeenCalledTimes(1);
    expect(lastSaved().layers.find((l) => l.id === "b")).toMatchObject({ sizing: "fit", rect: { x: 100, y: 100, w: 400, h: 200 } });
    await fireEvent.pointerUp(se(), { clientX: 400, clientY: 160, ctrlKey: true });
    await advance(500);
    // +100 canvas px = +200 screen px on a 400×200 box (no design: its own 2:1 kept).
    expect(lastSaved().layers.find((l) => l.id === "b")).toMatchObject({ sizing: "fit", rect: { x: 100, y: 100, w: 600, h: 300 } });
    expect(api.setLayerRect).toHaveBeenLastCalledWith("s1", "b", { x: 100, y: 100, w: 600, h: 300 });
    // Shift: free.
    await fireEvent.pointerDown(se(), { clientX: 400, clientY: 200, button: 0 });
    await fireEvent.pointerMove(se(), { clientX: 450, clientY: 200, shiftKey: true, ctrlKey: true });
    await fireEvent.pointerUp(se(), { clientX: 450, clientY: 200, shiftKey: true, ctrlKey: true });
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.rect).toEqual({ x: 100, y: 100, w: 700, h: 300 });
    // Moving never changes the sizing (layer c has none).
    await fireEvent.focus(box(container, "c"));
    await fireEvent.keyDown(box(container, "c"), { key: "ArrowRight" });
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "c")!.sizing).toBeUndefined();
  });

  it("a Reflow layer resizes freely; Shift keeps the aspect ratio", async () => {
    reflowB();
    const { container } = await mount();
    await fireEvent.click(main(container, "b"));
    const free = box(container, "b").querySelector<HTMLElement>('[data-handle="se"]')!;
    await fireEvent.pointerDown(free, { clientX: 300, clientY: 150, button: 0 });
    await fireEvent.pointerMove(free, { clientX: 400, clientY: 160, ctrlKey: true });
    await fireEvent.pointerUp(free, { clientX: 400, clientY: 160, ctrlKey: true });
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")).toMatchObject({ sizing: "reflow", rect: { x: 100, y: 100, w: 600, h: 220 } });
    // Shift: +100 canvas px = +200 screen px on the 600×220 box, its aspect kept.
    const se = box(container, "b").querySelector<HTMLElement>('[data-handle="se"]')!;
    await fireEvent.pointerDown(se, { clientX: 350, clientY: 160, button: 0 });
    await fireEvent.pointerMove(se, { clientX: 450, clientY: 170, shiftKey: true, ctrlKey: true });
    await fireEvent.pointerUp(se, { clientX: 450, clientY: 170, shiftKey: true, ctrlKey: true });
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")).toMatchObject({ sizing: "reflow", rect: { x: 100, y: 100, w: 800, h: 293 } });
  });

  it("Alt resizes from the centre; no side goes below 16 px", async () => {
    reflowB();
    const { container } = await mount();
    await fireEvent.click(main(container, "b"));
    const se = box(container, "b").querySelector<HTMLElement>('[data-handle="se"]')!;
    await fireEvent.pointerDown(se, { clientX: 300, clientY: 150, button: 0 });
    await fireEvent.pointerMove(se, { clientX: 350, clientY: 160, altKey: true, ctrlKey: true });
    await fireEvent.pointerUp(se, { clientX: 350, clientY: 160, altKey: true, ctrlKey: true });
    await advance(500);
    // +50 / +10 canvas px = +100 / +20 screen px on each side: centre (300, 200) kept.
    expect(lastSaved().layers.find((l) => l.id === "b")!.rect).toEqual({ x: 0, y: 80, w: 600, h: 240 });

    const se2 = box(container, "b").querySelector<HTMLElement>('[data-handle="se"]')!;
    await fireEvent.pointerDown(se2, { clientX: 300, clientY: 160, button: 0 });
    await fireEvent.pointerMove(se2, { clientX: -500, clientY: -500, ctrlKey: true });
    await fireEvent.pointerUp(se2, { clientX: -500, clientY: -500, ctrlKey: true });
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.rect).toEqual({ x: 0, y: 80, w: 16, h: 16 });
  });

  it("arrow keys nudge 1 px / 10 px with Shift", async () => {
    const { container } = await mount();
    const b = box(container, "b");
    await fireEvent.focus(b);
    await fireEvent.keyDown(b, { key: "ArrowRight" });
    await fireEvent.keyDown(b, { key: "ArrowDown", shiftKey: true });
    await advance(500);
    expect(api.saveScreen).toHaveBeenCalledTimes(1);
    expect(lastSaved().layers.find((l) => l.id === "b")!.rect).toEqual({ x: 101, y: 110, w: 400, h: 200 });
    expect(api.setLayerRect).toHaveBeenCalledWith("s1", "b", { x: 101, y: 100, w: 400, h: 200 });
  });

  it("nothing is scheduled at rest", async () => {
    await mount();
    await advance(1000);
    expect(vi.getTimerCount()).toBe(0);
  });
});

// ---- Inspector ------------------------------------------------------------------------------------

describe("inspector", () => {
  async function selectB() {
    const r = await mount();
    await fireEvent.click(main(r.container, "b"));
    await tick();
    return r;
  }

  it("overrides a variable, marks it, and Reset clears the override", async () => {
    await selectB();
    const inspector = screen.getByRole("complementary", { name: "Inspector" });
    const hex = within(inspector).getByLabelText("Accent") as HTMLInputElement;
    expect(hex.value).toBe("#22b8cf");
    expect(within(inspector).queryByText("Overrides template")).toBeNull();
    await fireEvent.input(hex, { target: { value: "#ff0000" } });
    expect(within(inspector).getByText("Overrides template")).toBeTruthy();
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.vars).toEqual({ accent: "#ff0000" });

    await fireEvent.click(within(inspector).getByRole("button", { name: "Reset Accent to the template value" }));
    expect(within(inspector).queryByText("Overrides template")).toBeNull();
    expect((within(inspector).getByLabelText("Accent") as HTMLInputElement).value).toBe("#22b8cf");
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.vars).toEqual({});
  });

  it("an invalid override blocks saving and says why", async () => {
    await selectB();
    const inspector = screen.getByRole("complementary", { name: "Inspector" });
    await fireEvent.input(within(inspector).getByLabelText("Accent"), { target: { value: "#zz" } });
    expect(within(inspector).getByText("Enter a color like #1a1a1a.")).toBeTruthy();
    expect(screen.getByText("Not saved: Layer b: Accent: Enter a color like #1a1a1a.")).toBeTruthy();
    await advance(2000);
    expect(api.saveScreen).not.toHaveBeenCalled();
  });

  it("edits field values, the rect and Fill screen", async () => {
    await selectB();
    const inspector = screen.getByRole("complementary", { name: "Inspector" });
    await fireEvent.input(within(inspector).getByLabelText("Title"), { target: { value: "Admiral" } });
    const x = within(inspector).getByLabelText("X") as HTMLInputElement;
    x.value = "250";
    await fireEvent.change(x);
    await advance(500);
    const b = lastSaved().layers.find((l) => l.id === "b")!;
    expect(b.fields).toEqual({ name: "Ada", title: "Admiral" });
    expect(b.rect).toEqual({ x: 250, y: 100, w: 400, h: 200 });
    await fireEvent.click(within(inspector).getByRole("button", { name: "Fill screen" }));
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.rect).toEqual({ x: 0, y: 0, w: 1920, h: 1080 });
  });

  it("switching the template keeps matching keys", async () => {
    await selectB();
    const inspector = screen.getByRole("complementary", { name: "Inspector" });
    await fireEvent.input(within(inspector).getByLabelText("Accent"), { target: { value: "#ff0000" } });
    await fireEvent.change(within(inspector).getByLabelText("Template"), { target: { value: "t2" } });
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")).toMatchObject({
      templateId: "t2",
      name: "Layer b",
      fields: { name: "Ada", title: "" },
      vars: { accent: "#ff0000" },
    });
  });
});

describe("inspector: variants", () => {
  /** News strap (t2) has three looks; layer b uses it. */
  function variantDoc(variant?: string): ScreenDoc {
    const d = makeDoc();
    d.layers[1] = { ...d.layers[1], templateId: "t2", ...(variant ? { variant } : {}) };
    return d;
  }

  beforeEach(() => {
    api.getTemplate.mockImplementation(async (id: string) => {
      const s = summaries.find((t) => t.id === id);
      if (!s) throw { code: "notFound", message: "Template not found" };
      const t = makeTemplate(id, s.name);
      if (id !== "t2") return t;
      const look = (vid: string, name: string) => ({ id: vid, name, vars: {}, html: "<div></div>", css: "" });
      return { ...t, defaultVariant: "full", variants: [look("full", "Full"), look("compact", "Compact"), look("split", "Split")] };
    });
    api.getScreen.mockImplementation(async () => stateFor(variantDoc()));
    api.setLayerVariant.mockImplementation(async (_s: string, layerId: string, variant: string) =>
      stateFor({ ...variantDoc(), layers: variantDoc().layers.map((l) => (l.id === layerId ? { ...l, variant } : l)) }),
    );
  });

  async function selectLayer(id: string) {
    const r = await mount();
    await fireEvent.click(main(r.container, id));
    await tick();
    const inspector = screen.getByRole("complementary", { name: "Inspector" });
    await vi.waitFor(() => within(inspector).getByLabelText("Template"));
    return { ...r, inspector };
  }

  const switchTitles = () => registry.list.map(titleOf).filter((t) => t.startsWith("Switch "));

  it("lists the template's variants under Template, the default marked", async () => {
    const { inspector } = await selectLayer("b");
    const select = within(inspector).getByLabelText("Variant") as HTMLSelectElement;
    expect([...select.options].map((o) => o.textContent)).toEqual(["Full (default)", "Compact", "Split"]);
    expect(select.value).toBe("full");
  });

  it("a template with one variant shows no Variant select", async () => {
    const { inspector } = await selectLayer("a");
    expect(within(inspector).queryByLabelText("Variant")).toBeNull();
  });

  it("choosing one calls setLayerVariant with a crossfade; the working copy takes it without a save", async () => {
    const { inspector } = await selectLayer("b");
    await fireEvent.change(within(inspector).getByLabelText("Variant"), { target: { value: "compact" } });
    await advance(0);
    expect(api.setLayerVariant).toHaveBeenCalledExactlyOnceWith("s1", "b", "compact", "crossfade");
    expect((within(inspector).getByLabelText("Variant") as HTMLSelectElement).value).toBe("compact");
    await advance(2000);
    expect(api.saveScreen).not.toHaveBeenCalled();
    // A later edit saves the variant along.
    await fireEvent.input(within(inspector).getByLabelText("Title"), { target: { value: "Admiral" } });
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.variant).toBe("compact");
  });

  it("saves pending edits before switching; a failure puts the select back with a toast", async () => {
    const { inspector } = await selectLayer("b");
    await fireEvent.input(within(inspector).getByLabelText("Title"), { target: { value: "Admiral" } });
    api.setLayerVariant.mockRejectedValueOnce({ code: "notFound", message: "Layer not found" });
    await fireEvent.change(within(inspector).getByLabelText("Variant"), { target: { value: "split" } });
    await advance(0);
    expect(api.saveScreen).toHaveBeenCalledOnce();
    expect(api.saveScreen.mock.invocationCallOrder[0]).toBeLessThan(api.setLayerVariant.mock.invocationCallOrder[0]);
    expect((within(inspector).getByLabelText("Variant") as HTMLSelectElement).value).toBe("full");
    expect(toasts.list.at(-1)?.message).toBe("Layer not found");
  });

  it("shows the variantMissing warning and keeps the missing id listed", async () => {
    api.getScreen.mockImplementation(async () => {
      const st = stateFor(variantDoc("gone"));
      return { ...st, warnings: [...st.warnings, { layerId: "b", code: "variantMissing" as const, message: "x" }] };
    });
    const { inspector } = await selectLayer("b");
    expect(within(inspector).getByText("Variant “gone” no longer exists, so the default shows. Choose a variant.")).toBeTruthy();
    const select = within(inspector).getByLabelText("Variant") as HTMLSelectElement;
    expect(select.value).toBe("gone");
    expect([...select.options].map((o) => o.textContent)).toContain("Missing (gone)");
  });

  it("registers Switch <layer> to <variant> while mounted; running one switches with a crossfade", async () => {
    const r = await mount();
    await vi.waitFor(() => expect(switchTitles()).toEqual(["Switch Layer b to Compact", "Switch Layer b to Split"]));
    const cmd = registry.list.find((c) => titleOf(c) === "Switch Layer b to Split")!;
    await runCommand(cmd.id);
    await advance(0);
    expect(api.setLayerVariant).toHaveBeenCalledWith("s1", "b", "split", "crossfade");
    // The current variant has no command; the others follow.
    expect(switchTitles()).toEqual(["Switch Layer b to Full", "Switch Layer b to Compact"]);
    r.unmount();
    expect(switchTitles()).toEqual([]);
  });
});

describe("inspector: source bindings", () => {
  const fieldsOf = (inspector: HTMLElement) => within(inspector).getByRole("region", { name: "Fields" });

  async function selectB() {
    const r = await mount();
    await fireEvent.click(main(r.container, "b"));
    await tick();
    return { ...r, inspector: screen.getByRole("complementary", { name: "Inspector" }) };
  }

  /** The screen with layer b's fields bound (as saved on disk). */
  function boundDoc(): ScreenDoc {
    const d = makeDoc();
    d.layers[1] = { ...d.layers[1], bindings: { name: { sourceId: "g1", column: "name" } } };
    return d;
  }

  it("sources load lazily: not at mount without bindings, once when a Bind menu opens", async () => {
    const { inspector } = await selectB();
    expect(api.listSources).not.toHaveBeenCalled();
    await fireEvent.click(within(inspector).getByRole("button", { name: "Bind Name to source" }));
    await advance(0);
    expect(api.listSources).toHaveBeenCalledTimes(1);
    await fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    await fireEvent.click(within(inspector).getByRole("button", { name: "Bind Title to source" }));
    await advance(0);
    expect(api.listSources).toHaveBeenCalledTimes(1);
  });

  it("bind: pick a source, then a column; the field turns read-only and the doc is saved", async () => {
    const { inspector } = await selectB();
    await fireEvent.click(within(inspector).getByRole("button", { name: "Bind Name to source" }));
    await advance(0);
    const menu = screen.getByRole("menu");
    await fireEvent.click(within(menu).getByRole("menuitem", { name: /Guests/ }));
    await tick();
    const columns = screen.getByRole("menu", { name: "Columns of Guests" });
    expect(within(columns).getAllByRole("menuitem").map((m) => m.textContent?.replace(/\s+/g, " ").trim())).toEqual([
      "Guests",
      "Name name",
      "Title title",
    ]);
    await fireEvent.click(within(columns).getByRole("menuitem", { name: /^Name/ }));
    await tick();
    expect(screen.queryByRole("menu")).toBeNull();
    // Read-only: the source's current value with "Source · Column", no text input.
    expect(within(inspector).getByText("Guests · Name")).toBeTruthy();
    expect(within(inspector).getByText("Grace Hopper")).toBeTruthy();
    const ro = within(fieldsOf(inspector)).getByLabelText("Name");
    expect(ro.tagName).toBe("SPAN");
    expect(ro.getAttribute("aria-readonly")).toBe("true");
    await advance(500);
    const b = lastSaved().layers.find((l) => l.id === "b")!;
    expect(b.bindings).toEqual({ name: { sourceId: "g1", column: "name" } });
    // The layer's own value is kept as the fallback.
    expect(b.fields.name).toBe("Ada");
  });

  it("the Bind menu is keyboard operable", async () => {
    const { inspector } = await selectB();
    const btn = within(inspector).getByRole("button", { name: "Bind Title to source" });
    await fireEvent.keyDown(btn, { key: "ArrowDown" });
    await advance(0);
    expect(document.activeElement?.textContent).toMatch(/Guests/);
    (document.activeElement as HTMLButtonElement).click(); // Enter/Space on a focused button
    await tick();
    await tick();
    // Columns step: Back first, then the columns.
    await fireEvent.keyDown(screen.getByRole("menu"), { key: "ArrowDown" });
    await fireEvent.keyDown(screen.getByRole("menu"), { key: "ArrowDown" });
    expect(document.activeElement?.textContent).toMatch(/Title/);
    (document.activeElement as HTMLButtonElement).click(); // Enter/Space on a focused button
    await tick();
    await advance(0);
    expect(document.activeElement).toBe(within(inspector).getByRole("button", { name: "Unbind Title" }));
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.bindings).toEqual({ title: { sourceId: "g1", column: "title" } });
  });

  it("a screen with bindings loads the sources at once; Unbind restores the input and saves", async () => {
    api.getScreen.mockImplementation(async () => stateFor(boundDoc()));
    const { inspector } = await selectB();
    expect(api.listSources).toHaveBeenCalledTimes(1);
    expect(within(inspector).getByText("Guests · Name")).toBeTruthy();
    await fireEvent.click(within(inspector).getByRole("button", { name: "Unbind Name" }));
    expect((within(fieldsOf(inspector)).getByLabelText("Name") as HTMLInputElement).value).toBe("Ada");
    await advance(500);
    expect("bindings" in lastSaved().layers.find((l) => l.id === "b")!).toBe(false);
  });

  it("shows the sourceMissing warning on the layer and in the Inspector", async () => {
    api.getScreen.mockImplementation(async () => {
      const st = stateFor(boundDoc());
      return {
        ...st,
        warnings: [...st.warnings, { layerId: "b", code: "sourceMissing" as const, message: "Source g1 not found" }],
      };
    });
    api.listSources.mockResolvedValue([]);
    const { container, inspector } = await selectB();
    expect(within(row(container, "b")).getByText("Source missing")).toBeTruthy();
    expect(within(row(container, "a")).queryByText("Source missing")).toBeNull();
    expect(within(inspector).getByText(/Source g1 not found/)).toBeTruthy();
    expect(within(inspector).getByText("Missing source · name")).toBeTruthy();
  });

  it("switching the template drops bindings for fields the new template doesn't have", async () => {
    const d = makeDoc();
    d.layers[1] = {
      ...d.layers[1],
      bindings: { name: { sourceId: "g1", column: "name" }, title: { sourceId: "g1", column: "title" } },
    };
    api.getScreen.mockImplementation(async () => stateFor(d));
    api.getTemplate.mockImplementation(async (id: string) => {
      const t = makeTemplate(id, id === "t2" ? "News strap" : "Classic bar");
      if (id === "t2") t.fields = [t.fields[0]];
      return t;
    });
    const { inspector } = await selectB();
    await fireEvent.change(within(inspector).getByLabelText("Template"), { target: { value: "t2" } });
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.bindings).toEqual({
      name: { sourceId: "g1", column: "name" },
    });
  });

  it("a source changed on disk refreshes the loaded list", async () => {
    api.getScreen.mockImplementation(async () => stateFor(boundDoc()));
    await selectB();
    expect(api.listSources).toHaveBeenCalledTimes(1);
    emit("project-files-changed", { kind: "source", id: "g1", change: "changed", message: null });
    await advance(0);
    expect(api.listSources).toHaveBeenCalledTimes(2);
    expect(api.getScreen).toHaveBeenCalledTimes(1);
  });
});

// ---- Header ---------------------------------------------------------------------------------------

describe("header", () => {
  it("background: colour + hex, saved as the doc background", async () => {
    await mount();
    await fireEvent.change(screen.getByLabelText("Background"), { target: { value: "colour" } });
    const hex = screen.getByLabelText("Background color") as HTMLInputElement;
    await fireEvent.input(hex, { target: { value: "#112233" } });
    await advance(500);
    expect(lastSaved().background).toBe("#112233");
    await fireEvent.change(screen.getByLabelText("Background"), { target: { value: "transparent" } });
    await advance(500);
    expect(lastSaved().background).toBe("transparent");
  });

  it("duplicate saves first, then opens the copy", async () => {
    api.duplicateScreen.mockResolvedValue(stateFor({ ...makeDoc(), id: "s2", name: "Main program copy" }));
    const { container } = await mount();
    await fireEvent.click(within(row(container, "b")).getByRole("button", { name: "Delete layer" }));
    await fireEvent.click(screen.getByRole("button", { name: "Duplicate" }));
    await advance(0);
    expect(api.saveScreen).toHaveBeenCalledTimes(1);
    expect(api.duplicateScreen).toHaveBeenCalledWith("s1", "Main program copy");
    expect(nav.goto).toHaveBeenCalledWith("/screens/s2");
  });

  it("delete asks first, deletes without saving and leaves", async () => {
    api.deleteScreen.mockResolvedValue(undefined);
    const { container } = await mount();
    await fireEvent.click(within(row(container, "b")).getByRole("button", { name: "Delete layer" }));
    await fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    await vi.waitFor(() => expect(screen.getByText("Delete screen?")).toBeTruthy());
    const dialog = screen.getByRole("dialog", { hidden: true });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Delete", hidden: true }));
    await advance(0);
    expect(api.deleteScreen).toHaveBeenCalledWith("s1");
    expect(nav.goto).toHaveBeenCalledWith("/screens");
    cleanup();
    expect(api.saveScreen).not.toHaveBeenCalled();
  });

  it("renames on Enter", async () => {
    api.renameScreen.mockImplementation(async (id: string, name: string) => ({
      id,
      name,
      width: 1920,
      height: 1080,
      background: "transparent",
      layerCount: 3,
      updatedAt: "2026-10-03T22:00:00Z",
    }));
    await mount();
    const input = screen.getByLabelText("Screen name") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "Stage left" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    await advance(0);
    expect(api.renameScreen).toHaveBeenCalledWith("s1", "Stage left");
  });

  it("unmount flushes a pending save", async () => {
    const { container, unmount } = await mount();
    await fireEvent.click(within(row(container, "b")).getByRole("button", { name: "Delete layer" }));
    unmount();
    expect(api.saveScreen).toHaveBeenCalledTimes(1);
    expect(events.handlers.get("project-files-changed")?.size ?? 0).toBe(0);
  });
});

// ---- External changes -------------------------------------------------------------------------------

describe("changed on disk", () => {
  const changed = (over: Partial<FilesChanged> = {}): FilesChanged => ({
    kind: "screen",
    id: "s1",
    change: "changed",
    message: null,
    ...over,
  });

  it("without unsaved edits: reloads silently with a toast", async () => {
    const { container } = await mount();
    api.getScreen.mockResolvedValueOnce(stateFor({ ...makeDoc(), layers: [layer("a")] }));
    emit("project-files-changed", changed());
    await advance(0);
    expect(panelOrder(container)).toEqual(["a"]);
    expect(screen.queryByRole("alert")).toBeNull();
    expect(toasts.list.at(-1)?.message).toMatch(/changed on disk and was reloaded/);
  });

  it("ignores other screens", async () => {
    await mount();
    emit("project-files-changed", changed({ id: "s9" }));
    await advance(0);
    expect(api.getScreen).toHaveBeenCalledTimes(1);
  });

  it("with unsaved edits: a banner; Keep mine saves over the file", async () => {
    const { container } = await mount();
    await fireEvent.click(within(row(container, "b")).getByRole("button", { name: "Delete layer" }));
    emit("project-files-changed", changed());
    await tick();
    const banner = await screen.findByRole("alert");
    expect(within(banner).getByText("Changed on disk.")).toBeTruthy();
    // Autosave waits for the choice.
    await advance(2000);
    expect(api.saveScreen).not.toHaveBeenCalled();
    await fireEvent.click(within(banner).getByRole("button", { name: "Keep mine" }));
    await advance(0);
    expect(api.saveScreen).toHaveBeenCalledTimes(1);
    expect(lastSaved().layers.map((l) => l.id)).toEqual(["a", "c"]);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("with unsaved edits: Reload replaces the working copy", async () => {
    const { container } = await mount();
    await fireEvent.click(within(row(container, "b")).getByRole("button", { name: "Delete layer" }));
    emit("project-files-changed", changed());
    await tick();
    api.getScreen.mockResolvedValueOnce(stateFor({ ...makeDoc(), layers: [layer("z")] }));
    await fireEvent.click(within(await screen.findByRole("alert")).getByRole("button", { name: "Reload" }));
    await advance(0);
    expect(panelOrder(container)).toEqual(["z"]);
    expect(screen.queryByRole("alert")).toBeNull();
    await advance(2000);
    expect(api.saveScreen).not.toHaveBeenCalled();
  });

  it("removed: a banner; Close goes back to the list, nothing is saved", async () => {
    const { container } = await mount();
    await fireEvent.click(within(row(container, "b")).getByRole("button", { name: "Delete layer" }));
    emit("project-files-changed", changed({ change: "removed" }));
    await tick();
    const banner = await screen.findByRole("alert");
    expect(within(banner).getByText("Deleted on disk.")).toBeTruthy();
    await fireEvent.click(within(banner).getByRole("button", { name: "Close" }));
    expect(nav.goto).toHaveBeenCalledWith("/screens");
    cleanup();
    await advance(2000);
    expect(api.saveScreen).not.toHaveBeenCalled();
  });

  it("invalid: a toast with the message; the working copy stays", async () => {
    const { container } = await mount();
    emit("project-files-changed", changed({ change: "invalid", message: "expected `}` at line 3" }));
    await tick();
    expect(toasts.list.at(-1)).toMatchObject({ kind: "error" });
    expect(toasts.list.at(-1)?.message).toMatch(/expected `}` at line 3/);
    expect(panelOrder(container)).toEqual(["c", "b", "a"]);
  });

  it("a used template changed on disk is refetched", async () => {
    await mount();
    api.getTemplate.mockClear();
    emit("project-files-changed", changed({ kind: "template", id: "t1" }));
    await advance(0);
    expect(api.getTemplate).toHaveBeenCalledWith("t1");
  });
});

// ---- List and timer fields ----------------------------------------------------------------------

describe("inspector: list and timer fields", () => {
  const TIMER0 = '{"run":false,"at":0,"base":0,"dir":"up","end":"stop"}';
  const TIMER1 = '{"run":true,"at":1000,"base":0,"dir":"up","end":"stop"}';

  beforeEach(() => {
    api.getTemplate.mockImplementation(async (id: string) => {
      const t = makeTemplate(id, "Board");
      t.fields = [
        ...t.fields,
        {
          key: "board",
          label: "Board",
          type: "list",
          default: "[]",
          columns: [
            { key: "name", label: "Name" },
            { key: "score", label: "Score" },
          ],
        },
        { key: "clock", label: "Clock", type: "timer", default: TIMER0 },
        { key: "time", label: "Time", type: "clock", default: "HH:mm", format: "HH:mm" },
      ];
      return t;
    });
  });

  async function selectB() {
    const r = await mount();
    await fireEvent.click(main(r.container, "b"));
    await advance(0);
    return { ...r, inspector: screen.getByRole("complementary", { name: "Inspector" }) };
  }

  it("a list binds the whole source with sort and limit; the label says so and the doc is saved", async () => {
    const { inspector } = await selectB();
    // A timer binds only a schedule's countdown; a clock never binds.
    expect(await within(inspector).findByRole("button", { name: "Bind Clock to source" })).toBeTruthy();
    expect(within(inspector).queryByRole("button", { name: "Bind Time to source" })).toBeNull();
    await fireEvent.click(await within(inspector).findByRole("button", { name: "Link Board to a data table…" }));
    await advance(0);
    expect(api.listSources).toHaveBeenCalledTimes(1);
    const dialog = screen.getByRole("dialog", { name: "Link Board to a data table" });
    expect((within(dialog).getByRole("combobox", { name: "Table" }) as HTMLSelectElement).value).toBe("g1");
    expect(within(dialog).getByText("2 rows · 1/2 columns match")).toBeTruthy();
    await fireEvent.change(within(dialog).getByRole("combobox", { name: "Sort by" }), { target: { value: "title" } });
    await fireEvent.click(within(dialog).getByRole("switch", { name: "Compare as numbers" }));
    await fireEvent.click(within(dialog).getByRole("switch", { name: "Descending" }));
    await fireEvent.click(within(dialog).getByRole("switch", { name: "Limit rows" }));
    await fireEvent.change(within(dialog).getByRole("spinbutton", { name: "Most rows" }), { target: { value: "5" } });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Link" }));
    await advance(0);
    expect(screen.queryByRole("dialog")).toBeNull();
    const linked = await within(inspector).findByRole("button", { name: "Edit data table Guests" });
    expect(linked.closest(".summary")?.getAttribute("title")).toBe("Guests · all rows, sorted by Title ↓, top 5");
    expect(document.activeElement).toBe(within(inspector).getByRole("button", { name: "Link options of Board" }));
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.bindings).toEqual({
      board: { sourceId: "g1", column: "", sort: { column: "title", numeric: true, desc: true }, limit: 5 },
    });
  });

  it("link options reopen with the current link; Escape closes; Unlink keeps a copy of the rows", async () => {
    const d = makeDoc();
    d.layers[1] = { ...d.layers[1], bindings: { board: { sourceId: "g1", column: "", limit: 3 } } };
    api.getScreen.mockImplementation(async () => stateFor(d));
    const { inspector } = await selectB();
    const linked = await within(inspector).findByRole("button", { name: "Edit data table Guests" });
    expect(linked.closest(".summary")?.textContent?.replace(/\s+/g, " ").trim()).toBe("Linked to: Guests · Edit source");
    expect(linked.closest(".summary")?.getAttribute("title")).toBe("Guests · all rows, top 3");
    const opts = await within(inspector).findByRole("button", { name: "Link options of Board" });
    await fireEvent.click(opts);
    await advance(0);
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByRole("switch", { name: "Limit rows" }).getAttribute("aria-checked")).toBe("true");
    expect((within(dialog).getByRole("spinbutton", { name: "Most rows" }) as HTMLInputElement).value).toBe("3");
    await fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(opts);
    api.getSource.mockResolvedValue({
      id: "g1",
      name: "Guests",
      kind: "text",
      columns: [{ key: "name", label: "Name" }, { key: "title", label: "Title" }],
      rows: [{ name: "Grace Hopper", title: "Admiral" }, { name: "Ada" }, { name: "Alan" }, { name: "Kate" }],
      index: 0,
      updatedAt: "",
    });
    await fireEvent.click(within(inspector).getByRole("button", { name: "Unlink Board" }));
    expect(await within(inspector).findByRole("button", { name: "Edit items of Board" })).toBeTruthy();
    await advance(500);
    expect("bindings" in lastSaved().layers.find((l) => l.id === "b")!).toBe(false);
    // The copy: the first 3 rows (the link's limit), only the field's columns.
    expect(lastSaved().layers.find((l) => l.id === "b")!.fields.board).toBe(
      '[{"name":"Grace Hopper"},{"name":"Ada"},{"name":"Alan"}]',
    );
  });

  it("a list is one summary line; rows edited in the drawer are saved as the field's JSON value", async () => {
    const { inspector } = await selectB();
    const edit = await within(inspector).findByRole("button", { name: "Edit items of Board" });
    expect(edit.closest(".summary")?.textContent?.replace(/\s+/g, " ").trim()).toBe("0 items · Edit");
    expect(within(inspector).queryByRole("table")).toBeNull();
    await fireEvent.click(edit);
    await advance(0);
    await vi.waitFor(() => expect(screen.getByRole("grid", { name: "Board" })).toBeTruthy(), { timeout: 5000 });
    await fireEvent.click(screen.getByRole("button", { name: "Stub add row" }));
    await fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await advance(0);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(edit.closest(".summary")?.textContent?.replace(/\s+/g, " ").trim()).toBe("1 item · Edit");
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.fields.board).toBe('[{"name":"New"}]');
  });

  it("a linked list opens its data table; a save reloads the sources", async () => {
    const d = makeDoc();
    d.layers[1] = { ...d.layers[1], bindings: { board: { sourceId: "g1", column: "" } } };
    api.getScreen.mockImplementation(async () => stateFor(d));
    api.getSource.mockResolvedValue({
      id: "g1",
      name: "Guests",
      kind: "text",
      columns: sourceList[0].columns,
      rows: [{ name: "Grace Hopper" }, { name: "Ada" }],
      index: 1,
      updatedAt: "",
    });
    api.saveSource.mockImplementation(async (doc: unknown) => doc);
    const { inspector } = await selectB();
    await fireEvent.click(await within(inspector).findByRole("button", { name: "Edit data table Guests" }));
    await advance(0);
    await vi.waitFor(() => expect(screen.getByRole("grid", { name: "Guests" })).toBeTruthy());
    expect(api.getSource).toHaveBeenCalledWith("g1");
    api.listSources.mockClear();
    await fireEvent.click(screen.getByRole("button", { name: "Stub add row" }));
    await advance(600);
    expect(api.saveSource).toHaveBeenCalledTimes(1);
    expect(api.saveSource.mock.calls[0][0].rows).toHaveLength(3);
    await advance(0);
    expect(api.listSources).toHaveBeenCalled();
    // The layer's own value is untouched.
    expect(api.saveScreen).not.toHaveBeenCalled();
  });

  it("timer controls call setTimer; the returned state replaces the field without a save", async () => {
    api.setTimer.mockImplementation(async (_s: string, id: string) => ({
      ...liveView(id, true),
      fields: { name: "Ada", title: "", clock: TIMER1 },
    }));
    const { inspector } = await selectB();
    await fireEvent.click(await within(inspector).findByRole("button", { name: "Start Clock" }));
    await advance(0);
    expect(api.setTimer).toHaveBeenCalledWith("s1", "b", "clock", { action: "start" });
    expect(api.saveScreen).not.toHaveBeenCalled();
    expect(within(inspector).getByRole("button", { name: "Pause Clock" })).toBeTruthy();
    // A later edit saves the server's timer state, not the old one.
    const fields = within(inspector).getByRole("region", { name: "Fields" });
    await fireEvent.input(within(fields).getByLabelText("Name"), { target: { value: "Bo" } });
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.fields.clock).toBe(TIMER1);
  });

  it("a timer changed elsewhere (live-changed) updates the working copy", async () => {
    const { inspector } = await selectB();
    await within(inspector).findByRole("button", { name: "Start Clock" });
    emit("live-changed", {
      screen: { id: "s1", name: "Main program", width: 1920, height: 1080, background: "transparent" },
      layers: [{ ...liveView("b", true), fields: { clock: TIMER1 } }],
    });
    await advance(0);
    expect(within(inspector).getByRole("button", { name: "Pause Clock" })).toBeTruthy();
    expect(api.saveScreen).not.toHaveBeenCalled();
  });
});

describe("inspector: schedule bindings", () => {
  const TIMER0 = '{"run":false,"at":0,"base":0,"dir":"up","end":"stop"}';
  const schedule: SourceSummary = {
    id: "r1",
    name: "Run of show",
    kind: "schedule",
    rowCount: 4,
    index: 0,
    current: {},
    columns: [],
    usedBy: [],
    auto: null,
    nextAt: null,
    nowId: "i1",
    nextId: "i2",
    updatedAt: "",
  };

  beforeEach(() => {
    api.listSources.mockResolvedValue([...sourceList, schedule]);
    api.getTemplate.mockImplementation(async (id: string) => {
      const t = makeTemplate(id, "Board");
      t.fields = [
        ...t.fields,
        { key: "agenda", label: "Agenda", type: "list", default: "[]", columns: [{ key: "title", label: "Title" }] },
        { key: "clock", label: "Clock", type: "timer", default: TIMER0 },
      ];
      return t;
    });
  });

  async function selectB() {
    const r = await mount();
    await fireEvent.click(main(r.container, "b"));
    await advance(0);
    return { ...r, inspector: screen.getByRole("complementary", { name: "Inspector" }) };
  }

  it("a single field binds a schedule column with the row selector (Now / Up next)", async () => {
    const { inspector } = await selectB();
    await fireEvent.click(within(inspector).getByRole("button", { name: "Bind Name to source" }));
    await advance(0);
    const menu = screen.getByRole("menu");
    expect(within(menu).getByRole("menuitem", { name: /Run of show/ }).textContent).toMatch(/4 items/);
    await fireEvent.click(within(menu).getByRole("menuitem", { name: /Run of show/ }));
    await tick();
    const radios = screen.getAllByRole("menuitemradio");
    expect(radios.map((r) => r.textContent?.replace(/[●○\s]+/g, " ").trim())).toEqual(["Now", "Up next"]);
    expect(radios[0].getAttribute("aria-checked")).toBe("true");
    const column = within(screen.getByRole("group", { name: "Column" }));
    const keys = column.getAllByRole("menuitem").map((m) => m.querySelector(".meta")?.textContent);
    expect(keys).toContain("title");
    expect(keys).toContain("new_start");
    expect(keys).not.toContain("countdown");
    await fireEvent.click(radios[1]);
    await tick();
    expect(screen.getAllByRole("menuitemradio")[1].getAttribute("aria-checked")).toBe("true");
    await fireEvent.click(column.getByRole("menuitem", { name: /^Title/ }));
    await tick();
    expect(within(inspector).getByText("Run of show · Up next · Title")).toBeTruthy();
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.bindings).toEqual({
      name: { sourceId: "r1", column: "title", row: "next" },
    });
  });

  it("a timer binds only a schedule's countdown", async () => {
    const { inspector } = await selectB();
    await fireEvent.click(within(inspector).getByRole("button", { name: "Bind Clock to source" }));
    await advance(0);
    const menu = screen.getByRole("menu");
    expect(within(menu).queryByRole("menuitem", { name: /Guests/ })).toBeNull();
    await fireEvent.click(within(menu).getByRole("menuitem", { name: /Run of show/ }));
    await tick();
    const items = screen.getAllByRole("menuitem").map((m) => m.textContent?.trim());
    expect(items).toEqual(["Run of show", "Up next · Countdown", "Now · Countdown"]);
    await fireEvent.click(screen.getByRole("menuitem", { name: "Up next · Countdown" }));
    await tick();
    // A bound timer shows only its binding (no timer controls).
    expect(within(inspector).getByText("Run of show · Up next · Countdown")).toBeTruthy();
    expect(within(inspector).getByRole("button", { name: "Unbind Clock" })).toBeTruthy();
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.bindings).toEqual({
      clock: { sourceId: "r1", column: "countdown", row: "next" },
    });
  });

  it("a timer with no schedule in the project says so", async () => {
    api.listSources.mockResolvedValue(sourceList);
    const { inspector } = await selectB();
    await fireEvent.click(within(inspector).getByRole("button", { name: "Bind Clock to source" }));
    await advance(0);
    expect(screen.getByRole("menu").textContent).toMatch(/no schedules yet/);
  });

  it("a list binds all items of a schedule", async () => {
    const { inspector } = await selectB();
    await fireEvent.click(await within(inspector).findByRole("button", { name: "Link Agenda to a data table…" }));
    await advance(0);
    const dialog = screen.getByRole("dialog", { name: "Link Agenda to a data table" });
    await fireEvent.change(within(dialog).getByRole("combobox", { name: "Table" }), { target: { value: "r1" } });
    await tick();
    expect(within(dialog).getByText("4 items · 1/1 columns match")).toBeTruthy();
    await fireEvent.click(within(dialog).getByRole("button", { name: "Link" }));
    await advance(0);
    expect(within(inspector).getByText("Run of show")).toBeTruthy();
    // A schedule is edited on its own page: no Edit here.
    expect(within(inspector).queryByRole("button", { name: /^Edit data table/ })).toBeNull();
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.bindings).toEqual({ agenda: { sourceId: "r1", column: "" } });
  });
});

// ---- Anchors ----------------------------------------------------------------------------------------

describe("anchors", () => {
  async function selectB() {
    const r = await mount();
    await fireEvent.click(main(r.container, "b"));
    await tick();
    return { ...r, inspector: screen.getByRole("complementary", { name: "Inspector" }) };
  }

  it("a grid cell anchors the layer: the box moves at once and the doc is saved with the anchor", async () => {
    const { container, inspector } = await selectB();
    await fireEvent.click(within(inspector).getByRole("button", { name: "Center" }));
    await advance(0);
    // Saved at once (no 500 ms wait), placed by the TS mirror; 960×540 stage = scale 0.5.
    expect(api.saveScreen).toHaveBeenCalledTimes(1);
    const b = lastSaved().layers.find((l) => l.id === "b")!;
    expect(b.anchor).toEqual({ h: "center", v: "middle", safe: false });
    expect(b.rect).toEqual({ x: 760, y: 440, w: 400, h: 200 });
    expect(box(container, "b").style.transform).toBe("translate(380px, 220px)");
    expect(within(inspector).getByRole("button", { name: "Center" }).getAttribute("aria-pressed")).toBe("true");
    // Respect safe area keeps the point and re-places nothing for a centred box.
    await fireEvent.click(within(inspector).getByRole("switch", { name: "Respect safe area" }));
    await fireEvent.click(within(inspector).getByRole("button", { name: "Bottom right" }));
    await advance(0);
    expect(lastSaved().layers.find((l) => l.id === "b")).toMatchObject({
      anchor: { h: "right", v: "bottom", safe: true },
      rect: { x: 1920 - 96 - 400, y: 1080 - 54 - 200, w: 400, h: 200 },
    });
  });

  it("a width change keeps it centred; a nudge releases the moved axis", async () => {
    const { container, inspector } = await selectB();
    await fireEvent.click(within(inspector).getByRole("button", { name: "Center horizontally" }));
    await advance(0);
    expect(lastSaved().layers.find((l) => l.id === "b")).toMatchObject({
      anchor: { h: "center", v: null, safe: false },
      rect: { x: 760, y: 100, w: 400, h: 200 },
    });
    const w = within(inspector).getByLabelText("Width") as HTMLInputElement;
    w.value = "600";
    await fireEvent.change(w);
    await advance(500);
    // A typed width is a resize: Scale to fit, the aspect kept (2:1, no design).
    expect(lastSaved().layers.find((l) => l.id === "b")).toMatchObject({ sizing: "fit", rect: { x: 660, y: 100, w: 600, h: 300 } });
    // Nudging down keeps the horizontal anchor; nudging right lets it go.
    const b = box(container, "b");
    await fireEvent.focus(b);
    await fireEvent.keyDown(b, { key: "ArrowDown" });
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.anchor).toEqual({ h: "center", v: null, safe: false });
    await fireEvent.keyDown(b, { key: "ArrowRight" });
    await advance(500);
    const saved = lastSaved().layers.find((l) => l.id === "b")!;
    expect(saved.rect).toEqual({ x: 661, y: 101, w: 600, h: 300 });
    expect("anchor" in saved).toBe(false);
  });
});

// The layout editor's precise tools as a user drives them: rulers and guides, toolbar toggles
// (ui-state), overlays (editor-only: nothing goes to the renderer), multi-selection, align /
// distribute, the aspect lock and the editor's palette commands.
import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi, onTestFinished } from "vitest";
import type { LiveLayerView, Project, ScreenDoc as AnyScreenDoc, TemplateScreenLayer as ScreenLayer, ScreenState, TemplateDoc, UiState } from "$lib/types";

const events = vi.hoisted(() => ({ handlers: new Map<string, Set<(p: unknown) => void>>() }));
const api = vi.hoisted(() => ({
  getScreen: vi.fn(),
  saveScreen: vi.fn(),
  setLayerRect: vi.fn(),
  renameScreen: vi.fn(),
  duplicateScreen: vi.fn(),
  deleteScreen: vi.fn(),
  liveShow: vi.fn(),
  liveHide: vi.fn(),
  serverStatus: vi.fn(),
  listTemplates: vi.fn(),
  getTemplate: vi.fn(),
  listSources: vi.fn(),
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  on: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e ? e : { code: "internal", message: String(e) },
}));
vi.mock("$lib/api/screens", () => api);
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/templates", () => api);
vi.mock("$lib/api/sources", () => api);
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$lib/components/templates/gallery/launcher.svelte", () => ({
  GalleryLauncher: class {
    component = null;
    open = false;
    show = () => {};
    close = () => {};
  },
}));

/** A visible 960×540 canvas at once: scale 0.5 for a 1920×1080 screen. */
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
import { registry } from "$lib/commands/registry.svelte";
import { project } from "$lib/stores/project.svelte";
import { server } from "$lib/stores/server.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { uiState } from "$lib/stores/uiState.svelte";
import { DEFAULT_SCREEN_EDITOR } from "./limits";
/** Test docs hold template layers only. */
type ScreenDoc = Omit<AnyScreenDoc, "layers"> & { layers: ScreenLayer[] };

// The editor's panels are lazy chunks: load them once up front, so a cold first import under a
// busy parallel run doesn't eat the first test's time.
beforeAll(async () => {
  await Promise.all([import("./LayersPanel.svelte"), import("./Toolbar.svelte"), import("./Inspector.svelte")]);
});

const template: TemplateDoc = {
  id: "t1",
  name: "Bar",
  description: "",
  preset: null,
  fields: [],
  variables: [],
  autoOutMs: null,
  updatedAt: "",
  defaultVariant: "default",
  variants: [{ id: "default", name: "Default", vars: {}, html: "<div></div>", css: "" }],
};

const layer = (id: string, rect: ScreenLayer["rect"]): ScreenLayer => ({
  id,
  name: `Layer ${id}`,
  kind: "template",
  templateId: "t1",
  rect,
  fields: {},
  vars: {},
});

let docOnDisk: ScreenDoc;
function makeDoc(over: Partial<ScreenDoc> = {}): ScreenDoc {
  return {
    id: "s1",
    name: "Wall",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [layer("a", { x: 100, y: 100, w: 200, h: 100 }), layer("b", { x: 600, y: 300, w: 400, h: 200 }), layer("c", { x: 1200, y: 700, w: 100, h: 100 })],
    updatedAt: "2026-10-03T20:00:00Z",
    ...over,
  };
}
const view = (id: string): LiveLayerView => ({ id, name: id, onAir: false, pending: false, fields: {}, fieldDefs: [] });
const stateFor = (doc: ScreenDoc): ScreenState => ({
  doc,
  live: {
    screen: { id: doc.id, name: doc.name, width: doc.width, height: doc.height, background: doc.background },
    layers: doc.layers.map((l) => view(l.id)),
  },
  warnings: [],
});

const theProject: Project = {
  path: "C:\\shows\\a",
  name: "A",
  createdAt: "",
  settings: { port: 7770, lanAccess: false, defaultResolution: { width: 1920, height: 1080 } },
};
const baseUi = (): UiState => ({
  sidebarPinned: false,
  control: { view: "grid", gridColumns: 2, selectedScreenId: null, hiddenScreenIds: [], liveEdit: "onTake", hotkeys: {}, automationOff: false },
  screenEditor: { ...DEFAULT_SCREEN_EDITOR },
});

async function advance(ms: number) {
  await vi.advanceTimersByTimeAsync(ms);
  await tick();
}

async function mount() {
  const r = render(ScreenEditor, { id: "s1" });
  await vi.waitFor(() => expect(r.container.querySelector("li[data-layer]")).toBeTruthy());
  // The toolbar, overlays and commands are lazy chunks.
  await vi.dynamicImportSettled();
  await advance(0);
  await vi.dynamicImportSettled();
  await tick();
  return r;
}

const box = (c: HTMLElement, id: string) => c.querySelector<HTMLElement>(`.box[data-layer="${id}"]`)!;
const lastSaved = (): ScreenDoc => api.saveScreen.mock.calls.at(-1)?.[0];
const savedRect = (id: string) => lastSaved().layers.find((l) => l.id === id)!.rect;
const docLayer = (id: string) => lastSaved().layers.find((l) => l.id === id)!;
const lastUi = (): UiState => api.setUiState.mock.calls.at(-1)?.[0];
const key = (init: KeyboardEventInit) => new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => setTimeout(() => cb(0), 16) as unknown as number);
  vi.stubGlobal("cancelAnimationFrame", (id: number) => clearTimeout(id));
  events.handlers.clear();
  toasts.clear();
  server.status = null;
  docOnDisk = makeDoc();
  api.on.mockImplementation((event: string, h: (p: unknown) => void) => {
    let set = events.handlers.get(event);
    if (!set) events.handlers.set(event, (set = new Set()));
    set.add(h);
    return () => set.delete(h);
  });
  api.getScreen.mockImplementation(async () => stateFor(docOnDisk));
  api.saveScreen.mockImplementation(async (doc: ScreenDoc) => stateFor({ ...doc, updatedAt: "2026-10-03T21:00:00Z" }));
  api.setLayerRect.mockImplementation(async (_s: string, id: string) => view(id));
  api.serverStatus.mockResolvedValue({ state: "running", port: 7770, url: "http://127.0.0.1:7770", lanUrl: null });
  api.listTemplates.mockResolvedValue([{ id: "t1", name: "Bar", description: "", preset: null, fieldCount: 0, variableCount: 0, updatedAt: "" }]);
  api.getTemplate.mockResolvedValue(template);
  api.setUiState.mockImplementation(async (u: UiState) => u);
  project.current = theProject;
  uiState.set(baseUi());
});

afterEach(() => {
  cleanup();
  project.current = null;
  uiState.set(null);
  vi.useRealTimers();
});

// ---- Rulers and guides ----------------------------------------------------------------------------

describe("rulers and guides", () => {
  it("dragging from the top ruler creates a horizontal guide, saved in the screen; nothing goes live", async () => {
    const { container } = await mount();
    const ruler = container.querySelector<HTMLElement>(".ruler.top")!;
    expect(ruler).toBeTruthy();
    await fireEvent.pointerDown(ruler, { clientX: 300, clientY: -10, button: 0 });
    await fireEvent.pointerMove(ruler, { clientX: 300, clientY: 150 });
    await advance(16);
    // While dragging: the guide follows with its position in mono.
    expect(container.querySelector(".guide.y.moving .pos")?.textContent).toBe("300");
    await fireEvent.pointerUp(ruler, { clientX: 300, clientY: 150 });
    await tick();
    expect(container.querySelector('[data-guide="y:300"]')).toBeTruthy();
    await advance(500);
    expect(lastSaved().guides).toEqual({ x: [], y: [300] });
    expect(api.setLayerRect).not.toHaveBeenCalled();
  });

  it("the left ruler makes vertical guides; dropping a guide back on its ruler deletes it", async () => {
    docOnDisk = makeDoc({ guides: { x: [960], y: [540] } });
    const { container } = await mount();
    const g = container.querySelector<HTMLElement>('[data-guide="x:960"]')!;
    await fireEvent.pointerDown(g, { clientX: 480, clientY: 100, button: 0 });
    await fireEvent.pointerMove(g, { clientX: -8, clientY: 100 });
    await fireEvent.pointerUp(g, { clientX: -8, clientY: 100 });
    await tick();
    expect(container.querySelector('[data-guide="x:960"]')).toBeNull();
    await advance(500);
    expect(lastSaved().guides).toEqual({ x: [], y: [540] });

    // Remove the last one: the doc drops the key, as the file omits it.
    const h = container.querySelector<HTMLElement>('[data-guide="y:540"]')!;
    await fireEvent.pointerDown(h, { clientX: 100, clientY: 270, button: 0 });
    await fireEvent.pointerUp(h, { clientX: 100, clientY: -3 });
    await advance(500);
    expect("guides" in lastSaved()).toBe(false);
  });

  it("double-click a guide to type its position: Enter applies, Esc cancels", async () => {
    docOnDisk = makeDoc({ guides: { x: [], y: [540] } });
    const { container } = await mount();
    await fireEvent.dblClick(container.querySelector('[data-guide="y:540"]')!);
    await tick();
    const input = screen.getByLabelText("Horizontal guide position in px") as HTMLInputElement;
    expect(input.classList.contains("mono")).toBe(true);
    expect(input.value).toBe("540");
    await fireEvent.input(input, { target: { value: "720" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    await tick();
    expect(container.querySelector('[data-guide="y:720"]')).toBeTruthy();

    await fireEvent.dblClick(container.querySelector('[data-guide="y:720"]')!);
    await tick();
    const again = screen.getByLabelText("Horizontal guide position in px") as HTMLInputElement;
    await fireEvent.input(again, { target: { value: "100" } });
    await fireEvent.keyDown(again, { key: "Escape" });
    await tick();
    expect(screen.queryByLabelText("Horizontal guide position in px")).toBeNull();
    expect(container.querySelector('[data-guide="y:720"]')).toBeTruthy();
    await advance(500);
    expect(lastSaved().guides).toEqual({ x: [], y: [720] });
  });

  it("boxes snap to guides (6 CSS px at this zoom = 12 screen px), not with Alt", async () => {
    docOnDisk = makeDoc({ guides: { x: [1000], y: [] } });
    const { container } = await mount();
    const a = box(container, "a");
    // a: x 100..300. Move right by 349 canvas px = 698 screen px: left edge at 798, right at 998 → 1000.
    await fireEvent.pointerDown(a, { clientX: 100, clientY: 75, button: 0 });
    await fireEvent.pointerMove(a, { clientX: 449, clientY: 75 });
    await advance(16);
    await fireEvent.pointerUp(a, { clientX: 449, clientY: 75 });
    await advance(500);
    expect(savedRect("a").x + savedRect("a").w).toBe(1000);
  });
});

// ---- Toolbar, ui-state and overlays ---------------------------------------------------------------

describe("toolbar and overlays", () => {
  it("toggles write the project's ui-state (optimistic) and draw editor-only overlays", async () => {
    const { container } = await mount();
    const toolbar = screen.getByRole("toolbar", { name: "Layout tools" });
    const grid = within(toolbar).getByRole("button", { name: "Grid (Ctrl+G)" });
    expect(grid.getAttribute("aria-pressed")).toBe("false");
    await fireEvent.click(grid);
    expect(grid.getAttribute("aria-pressed")).toBe("true");
    expect(lastUi().screenEditor).toMatchObject({ grid: true, gridSize: 16 });
    expect(container.querySelector('[data-overlay="grid"]')).toBeTruthy();

    await fireEvent.change(within(toolbar).getByLabelText("Grid size"), { target: { value: "32" } });
    expect(lastUi().screenEditor?.gridSize).toBe(32);
    await fireEvent.change(within(toolbar).getByLabelText("Grid size"), { target: { value: "custom" } });
    await tick();
    const custom = within(toolbar).getByLabelText("Custom grid size") as HTMLInputElement;
    custom.value = "10";
    await fireEvent.change(custom);
    expect(lastUi().screenEditor?.gridSize).toBe(10);

    await fireEvent.click(within(toolbar).getByRole("button", { name: "Action safe 93% (Ctrl+Shift+S)" }));
    await fireEvent.click(within(toolbar).getByRole("button", { name: "Title safe 90%" }));
    await fireEvent.click(within(toolbar).getByRole("button", { name: "Center cross" }));
    await fireEvent.change(within(toolbar).getByLabelText(/Crop overlay/), { target: { value: "9:16" } });
    await tick();
    for (const o of ["action-safe", "title-safe", "center-cross", "crop"]) {
      expect(container.querySelector(`[data-overlay="${o}"]`), o).toBeTruthy();
    }
    expect(lastUi().screenEditor).toMatchObject({ actionSafe: true, titleSafe: true, centerCross: true, crop: "9:16" });

    // Editor-only: nothing was sent to the output or saved in the screen, and still one iframe.
    await advance(2000);
    expect(api.setLayerRect).not.toHaveBeenCalled();
    expect(api.saveScreen).not.toHaveBeenCalled();
    expect(container.querySelectorAll("iframe")).toHaveLength(1);
    expect(container.querySelector("iframe")?.getAttribute("src")).toBe("http://127.0.0.1:7770/screen/s1?mute=1");
  });

  it("a refused ui-state write rolls back and says why", async () => {
    api.setUiState.mockRejectedValueOnce({ code: "io", message: "disk full" });
    await mount();
    const snap = screen.getByRole("button", { name: "Snapping (Ctrl+;)" });
    expect(snap.getAttribute("aria-pressed")).toBe("true");
    await fireEvent.click(snap);
    await advance(0);
    expect(snap.getAttribute("aria-pressed")).toBe("true");
    expect(toasts.list.at(-1)?.message).toBe("disk full");
  });

  it("custom safe areas: add, rename, remove in the panel; saved in the screen", async () => {
    const { container } = await mount();
    await fireEvent.click(screen.getByRole("button", { name: "Edit custom safe areas" }));
    await vi.dynamicImportSettled();
    await tick();
    const panel = screen.getByRole("region", { name: "Custom safe areas" });
    await fireEvent.click(within(panel).getByRole("button", { name: "Add safe area" }));
    await tick();
    const name = within(panel).getByLabelText("Safe area name") as HTMLInputElement;
    await fireEvent.input(name, { target: { value: "Left bezel" } });
    expect(container.querySelector('[data-overlay="safe-area"]')).toBeTruthy();
    await advance(500);
    const saved = lastSaved().safeAreas!;
    expect(saved).toHaveLength(1);
    expect(saved[0]).toMatchObject({ name: "Left bezel", rect: { x: 480, y: 270, w: 960, h: 540 } });
    expect(saved[0].id).toMatch(/^[A-Za-z0-9_-]{10}$/);

    // An empty name blocks saving and says why.
    await fireEvent.input(name, { target: { value: " " } });
    expect(screen.getByText("Not saved: Safe areas: every safe area needs a name.")).toBeTruthy();
    await fireEvent.click(within(panel).getByRole("button", { name: /^Remove/ }));
    await advance(500);
    expect("safeAreas" in lastSaved()).toBe(false);
    expect(api.setLayerRect).not.toHaveBeenCalled();
  });
});

// ---- Selection, align, distribute -----------------------------------------------------------------

describe("multi-selection", () => {
  it("Shift-click on the canvas and Ctrl-click in the panel add to the selection", async () => {
    const { container } = await mount();
    await fireEvent.pointerDown(box(container, "a"), { clientX: 60, clientY: 60, button: 0 });
    await fireEvent.pointerUp(box(container, "a"), { clientX: 60, clientY: 60 });
    await fireEvent.pointerDown(box(container, "b"), { clientX: 400, clientY: 200, button: 0, shiftKey: true });
    await fireEvent.pointerUp(box(container, "b"), { clientX: 400, clientY: 200 });
    await tick();
    expect(box(container, "a").hasAttribute("data-selected")).toBe(true);
    expect(box(container, "b").hasAttribute("data-selected")).toBe(true);
    await fireEvent.click(container.querySelector('li[data-layer="c"] .main')!, { ctrlKey: true });
    await vi.dynamicImportSettled();
    await tick();
    const inspector = screen.getByRole("complementary", { name: "Inspector" });
    expect(within(inspector).getByText("layers", { exact: false }).textContent).toContain("3");
    expect(container.querySelector(".bounds .tag")?.textContent).toBe("3 layers");
    // Shift-click again removes.
    await fireEvent.pointerDown(box(container, "c"), { clientX: 620, clientY: 370, button: 0, shiftKey: true });
    expect(box(container, "c").hasAttribute("data-selected")).toBe(false);
  });

  it("dragging a selection moves all of it: <= 20 setLayerRect/s in total, one save after the drop", async () => {
    const { container } = await mount();
    await fireEvent.pointerDown(box(container, "a"), { clientX: 60, clientY: 60, button: 0 });
    await fireEvent.pointerUp(box(container, "a"), { clientX: 60, clientY: 60 });
    await fireEvent.pointerDown(box(container, "b"), { clientX: 400, clientY: 200, button: 0, shiftKey: true });
    await fireEvent.pointerUp(box(container, "b"), { clientX: 400, clientY: 200 });
    api.setLayerRect.mockClear();
    const b = box(container, "b");
    await fireEvent.pointerDown(b, { clientX: 400, clientY: 200, button: 0 });
    for (let i = 1; i <= 250; i++) {
      await fireEvent.pointerMove(b, { clientX: 400 + i / 5, clientY: 200, altKey: true });
      await vi.advanceTimersByTimeAsync(4);
    }
    await vi.advanceTimersByTimeAsync(16);
    const during = api.setLayerRect.mock.calls.length;
    expect(during).toBeGreaterThan(4);
    expect(during).toBeLessThanOrEqual(21);
    expect(new Set(api.setLayerRect.mock.calls.map((c) => c[1]))).toEqual(new Set(["a", "b"]));
    await fireEvent.pointerUp(b, { clientX: 450, clientY: 200, altKey: true });
    await advance(500);
    expect(api.saveScreen).toHaveBeenCalledTimes(1);
    // 50 canvas px = 100 screen px for both.
    expect(savedRect("a")).toEqual({ x: 200, y: 100, w: 200, h: 100 });
    expect(savedRect("b")).toEqual({ x: 700, y: 300, w: 400, h: 200 });
    expect(savedRect("c")).toEqual({ x: 1200, y: 700, w: 100, h: 100 });
  });

  it("a marquee on empty canvas selects what it touches; arrows nudge the whole selection", async () => {
    const { container } = await mount();
    const overlay = container.querySelector<HTMLElement>(".overlay")!;
    // Screen 0..700 × 0..400 touches a and b, not c.
    await fireEvent.pointerDown(overlay, { clientX: 1, clientY: 1, button: 0 });
    await fireEvent.pointerMove(overlay, { clientX: 350, clientY: 200 });
    await advance(16);
    expect(container.querySelector(".marquee")).toBeTruthy();
    await fireEvent.pointerUp(overlay, { clientX: 350, clientY: 200 });
    await tick();
    expect(box(container, "a").hasAttribute("data-selected")).toBe(true);
    expect(box(container, "b").hasAttribute("data-selected")).toBe(true);
    expect(box(container, "c").hasAttribute("data-selected")).toBe(false);
    await fireEvent.keyDown(box(container, "a"), { key: "ArrowRight", shiftKey: true });
    await advance(500);
    expect(savedRect("a").x).toBe(110);
    expect(savedRect("b").x).toBe(610);
    expect(savedRect("c").x).toBe(1200);
  });

  it("align buttons align to the selection's bounds and save that doc; distribute needs three", async () => {
    const { container } = await mount();
    for (const id of ["a", "b", "c"]) {
      await fireEvent.click(container.querySelector(`li[data-layer="${id}"] .main`)!, { shiftKey: true });
    }
    await vi.dynamicImportSettled();
    await tick();
    const inspector = screen.getByRole("complementary", { name: "Inspector" });
    // Distribute horizontally: a 100..300, c 1200..1300 stay; b (400 wide) gets equal gaps of 250.
    await fireEvent.click(within(inspector).getByRole("button", { name: "Distribute horizontally (Alt+Shift+H)" }));
    await vi.dynamicImportSettled();
    await advance(500);
    expect(["a", "b", "c"].map((id) => savedRect(id).x)).toEqual([100, 550, 1200]);
    await fireEvent.click(within(inspector).getByRole("button", { name: "Align left (Alt+A)" }));
    await vi.dynamicImportSettled();
    await advance(500);
    expect(["a", "b", "c"].map((id) => savedRect(id).x)).toEqual([100, 100, 100]);
    await fireEvent.click(within(inspector).getByRole("button", { name: "Align bottom (Alt+S)" }));
    await vi.dynamicImportSettled();
    await advance(500);
    // Bounds bottom: 800 (c at 700 + 100).
    expect(["a", "b", "c"].map((id) => savedRect(id).y)).toEqual([700, 600, 700]);
    expect(api.saveScreen).toHaveBeenCalledTimes(3);
    // The bounds' X moves the whole group; W / H are read-only.
    const x = within(inspector).getByLabelText("Selection X") as HTMLInputElement;
    expect(x.value).toBe("100");
    expect((within(inspector).getByLabelText("Selection width") as HTMLInputElement).disabled).toBe(true);
    x.value = "150";
    await fireEvent.change(x);
    await advance(500);
    expect(["a", "b", "c"].map((id) => savedRect(id).x)).toEqual([150, 150, 150]);
  });

  it("one layer aligns to the screen", async () => {
    const { container } = await mount();
    await fireEvent.click(container.querySelector('li[data-layer="b"] .main')!);
    await vi.dynamicImportSettled();
    await tick();
    const inspector = screen.getByRole("complementary", { name: "Inspector" });
    await fireEvent.click(within(inspector).getByRole("button", { name: "Align center to the screen (Alt+H)" }));
    await vi.dynamicImportSettled();
    await advance(500);
    expect(savedRect("b")).toEqual({ x: 760, y: 300, w: 400, h: 200 });
    expect(api.setLayerRect).toHaveBeenCalledWith("s1", "b", { x: 760, y: 300, w: 400, h: 200 });
  });
});

// ---- Free dragging -----------------------------------------------------------------------------------

describe("free dragging", () => {
  const readout = (c: HTMLElement) => c.querySelector<HTMLElement>("[data-readout]");
  const down = (el: HTMLElement, x: number, y: number, init: MouseEventInit = {}) =>
    fireEvent.pointerDown(el, { clientX: x, clientY: y, button: 0, ...init });
  const winKey = (type: "keydown" | "keyup", init: KeyboardEventInit) => window.dispatchEvent(new KeyboardEvent(type, { bubbles: true, cancelable: true, ...init }));

  it("Ctrl+click on the canvas adds and removes like Shift+click", async () => {
    const { container } = await mount();
    await down(box(container, "a"), 60, 60);
    await fireEvent.pointerUp(box(container, "a"), { clientX: 60, clientY: 60 });
    await down(box(container, "b"), 400, 200, { ctrlKey: true });
    await fireEvent.pointerUp(box(container, "b"), { clientX: 400, clientY: 200 });
    await tick();
    expect(["a", "b", "c"].map((id) => box(container, id).hasAttribute("data-selected"))).toEqual([true, true, false]);
    await down(box(container, "a"), 60, 60, { ctrlKey: true });
    await tick();
    expect(["a", "b", "c"].map((id) => box(container, id).hasAttribute("data-selected"))).toEqual([false, true, false]);
  });

  it("shows a mono X / Y readout next to the box while moving; it hides on drop", async () => {
    const { container } = await mount();
    const a = box(container, "a");
    await down(a, 60, 60);
    expect(readout(container)).toBeNull();
    // +50 / +25 canvas px = +100 / +50 screen px, Alt: free.
    await fireEvent.pointerMove(a, { clientX: 110, clientY: 85, altKey: true });
    await advance(16);
    const r = readout(container)!;
    expect(r.textContent).toBe("X 200 Y 150");
    expect(r.classList.contains("mono")).toBe(true);
    // Under the box's bottom-left corner: (200, 250) screen px at scale 0.5, 4 px below.
    expect(r.style.transform).toBe("translate(100px, 129px) translate(0%, 0%)");
    // Updated in the same frame as the box.
    await fireEvent.pointerMove(a, { clientX: 111, clientY: 86, altKey: true });
    await advance(16);
    expect(readout(container)!.textContent).toBe("X 202 Y 152");
    expect(box(container, "a").style.transform).toBe("translate(101px, 76px)");
    await fireEvent.pointerUp(a, { clientX: 111, clientY: 86, altKey: true });
    await tick();
    expect(readout(container)).toBeNull();
    await advance(500);
    expect(savedRect("a")).toEqual({ x: 202, y: 152, w: 200, h: 100 });
  });

  it("a plain click shows no readout", async () => {
    const { container } = await mount();
    const a = box(container, "a");
    await down(a, 60, 60);
    await fireEvent.pointerMove(a, { clientX: 61, clientY: 60 });
    await advance(16);
    expect(readout(container)).toBeNull();
    await fireEvent.pointerUp(a, { clientX: 61, clientY: 60 });
    await advance(500);
    expect(api.saveScreen).not.toHaveBeenCalled();
  });

  it("shows W / H while resizing", async () => {
    const { container } = await mount();
    await fireEvent.click(container.querySelector('li[data-layer="b"] .main')!);
    await tick();
    const se = box(container, "b").querySelector<HTMLElement>('[data-handle="se"]')!;
    await down(se, 500, 250);
    // Shift: free (the layer scales, so it would keep its aspect otherwise).
    await fireEvent.pointerMove(se, { clientX: 600, clientY: 260, ctrlKey: true, shiftKey: true });
    await advance(16);
    expect(readout(container)!.textContent).toBe("W 600 H 220");
    await fireEvent.pointerUp(se, { clientX: 600, clientY: 260, ctrlKey: true, shiftKey: true });
    await tick();
    expect(readout(container)).toBeNull();
  });

  it("a multi-selection drags together; the readout shows the selection's bounds", async () => {
    const { container } = await mount();
    await down(box(container, "a"), 60, 60);
    await fireEvent.pointerUp(box(container, "a"), { clientX: 60, clientY: 60 });
    await down(box(container, "b"), 400, 200, { shiftKey: true });
    await fireEvent.pointerUp(box(container, "b"), { clientX: 400, clientY: 200 });
    const b = box(container, "b");
    await down(b, 400, 200);
    await fireEvent.pointerMove(b, { clientX: 430, clientY: 210, altKey: true });
    await advance(16);
    // Bounds a..b: (100, 100) → +60 / +20 screen px.
    expect(readout(container)!.textContent).toBe("X 160 Y 120");
    await fireEvent.pointerUp(b, { clientX: 430, clientY: 210, altKey: true });
    await advance(500);
    expect(savedRect("a")).toEqual({ x: 160, y: 120, w: 200, h: 100 });
    expect(savedRect("b")).toEqual({ x: 660, y: 320, w: 400, h: 200 });
  });

  it("Alt drags freely: no line snap, no guide; pressing / releasing it mid-drag applies at once", async () => {
    docOnDisk = makeDoc({ guides: { x: [1000], y: [] } });
    const { container } = await mount();
    const a = box(container, "a");
    // a: x 100..300. +349 canvas px = +698 screen px: right edge at 998, 2 px from the guide.
    await down(a, 100, 75);
    await fireEvent.pointerMove(a, { clientX: 449, clientY: 75 });
    await advance(16);
    expect(readout(container)!.textContent).toBe("X 800 Y 100");
    // Snap lines are the overlay's own `.guide`s (ruler guides live in the Rulers).
    expect(container.querySelector(".overlay > .guide.x")).toBeTruthy();
    // Alt down without moving: free at once.
    winKey("keydown", { key: "Alt", altKey: true });
    await advance(16);
    expect(readout(container)!.textContent).toBe("X 798 Y 100");
    expect(container.querySelector(".overlay > .guide")).toBeNull();
    // Released: snaps again.
    winKey("keyup", { key: "Alt" });
    await advance(16);
    expect(readout(container)!.textContent).toBe("X 800 Y 100");
    winKey("keydown", { key: "Alt", altKey: true });
    await advance(16);
    await fireEvent.pointerUp(a, { clientX: 449, clientY: 75, altKey: true });
    await advance(500);
    expect(savedRect("a").x).toBe(798);
    // The drag's key listeners are gone with it.
    winKey("keyup", { key: "Alt" });
    await advance(16);
    expect(readout(container)).toBeNull();
  });

  it("arrows nudge a multi-selection 1 px, Shift+arrows 10 px, through setLayerRect and one save", async () => {
    const { container } = await mount();
    await down(box(container, "a"), 60, 60);
    await fireEvent.pointerUp(box(container, "a"), { clientX: 60, clientY: 60 });
    await down(box(container, "c"), 620, 370, { shiftKey: true });
    await fireEvent.pointerUp(box(container, "c"), { clientX: 620, clientY: 370 });
    api.setLayerRect.mockClear();
    const a = box(container, "a");
    await fireEvent.keyDown(a, { key: "ArrowLeft" });
    await fireEvent.keyDown(a, { key: "ArrowUp", shiftKey: true });
    await advance(500);
    expect(savedRect("a")).toEqual({ x: 99, y: 90, w: 200, h: 100 });
    expect(savedRect("c")).toEqual({ x: 1199, y: 690, w: 100, h: 100 });
    expect(savedRect("b")).toEqual({ x: 600, y: 300, w: 400, h: 200 });
    expect(api.saveScreen).toHaveBeenCalledTimes(1);
    expect(api.setLayerRect).toHaveBeenCalledWith("s1", "a", { x: 99, y: 100, w: 200, h: 100 });
    expect(api.setLayerRect).toHaveBeenLastCalledWith("s1", "c", { x: 1199, y: 690, w: 100, h: 100 });
  });

  it("a held arrow key stays <= 20 setLayerRect/s and the last position goes live", async () => {
    const { container } = await mount();
    const b = box(container, "b");
    await fireEvent.focus(b);
    api.setLayerRect.mockClear();
    // Key repeat at ~30/s for one second.
    for (let i = 0; i < 30; i++) {
      await fireEvent.keyDown(b, { key: "ArrowRight", repeat: i > 0 });
      await vi.advanceTimersByTimeAsync(33);
    }
    await advance(500);
    expect(api.setLayerRect.mock.calls.length).toBeLessThanOrEqual(21);
    expect(api.setLayerRect).toHaveBeenLastCalledWith("s1", "b", { x: 630, y: 300, w: 400, h: 200 });
    expect(savedRect("b").x).toBe(630);
  });

  it("a marquee focuses the selection, so arrows nudge it at once", async () => {
    const { container } = await mount();
    const overlay = container.querySelector<HTMLElement>(".overlay")!;
    await fireEvent.pointerDown(overlay, { clientX: 1, clientY: 1, button: 0 });
    await fireEvent.pointerMove(overlay, { clientX: 350, clientY: 200 });
    await advance(16);
    await fireEvent.pointerUp(overlay, { clientX: 350, clientY: 200 });
    await advance(0);
    expect(document.activeElement).toBe(box(container, "a"));
    await fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    await advance(500);
    expect([savedRect("a").y, savedRect("b").y, savedRect("c").y]).toEqual([101, 301, 700]);
  });
});

// ---- Aspect lock ------------------------------------------------------------------------------------

describe("aspect lock", () => {
  it("a layer that scales is locked by default: W / H and the canvas keep the ratio; Unlock unlinks", async () => {
    const { container } = await mount();
    await fireEvent.click(container.querySelector('li[data-layer="b"] .main')!);
    await vi.dynamicImportSettled();
    await tick();
    const inspector = screen.getByRole("complementary", { name: "Inspector" });
    expect(within(inspector).getByRole("button", { name: "Unlock aspect ratio" }).getAttribute("aria-pressed")).toBe("true");
    const w = within(inspector).getByLabelText("Width") as HTMLInputElement;
    w.value = "800";
    await fireEvent.change(w);
    await advance(500);
    expect(savedRect("b")).toEqual({ x: 600, y: 300, w: 800, h: 400 });
    expect(docLayer("b").sizing).toBe("fit");

    // Canvas: the se handle +100 canvas px → +200 screen px wide, height follows (2:1).
    const se = box(container, "b").querySelector<HTMLElement>('[data-handle="se"]')!;
    await fireEvent.pointerDown(se, { clientX: 700, clientY: 350, button: 0 });
    await fireEvent.pointerMove(se, { clientX: 800, clientY: 355, ctrlKey: true });
    await fireEvent.pointerUp(se, { clientX: 800, clientY: 355, ctrlKey: true });
    await advance(500);
    expect(savedRect("b")).toEqual({ x: 600, y: 300, w: 1000, h: 500 });

    // Unlocked: W alone.
    await fireEvent.click(within(inspector).getByRole("button", { name: "Unlock aspect ratio" }));
    expect(within(inspector).getByRole("button", { name: "Lock aspect ratio" }).getAttribute("aria-pressed")).toBe("false");
    w.value = "900";
    await fireEvent.change(w);
    await advance(500);
    expect(savedRect("b")).toEqual({ x: 600, y: 300, w: 900, h: 500 });
  });

  it("a Reflow layer is unlocked by default; the lock keeps W / H", async () => {
    docOnDisk = makeDoc();
    docOnDisk.layers = docOnDisk.layers.map((l) => (l.id === "b" ? { ...l, sizing: "reflow" } : l));
    const { container } = await mount();
    await fireEvent.click(container.querySelector('li[data-layer="b"] .main')!);
    await vi.dynamicImportSettled();
    await tick();
    const inspector = screen.getByRole("complementary", { name: "Inspector" });
    const w = within(inspector).getByLabelText("Width") as HTMLInputElement;
    w.value = "500";
    await fireEvent.change(w);
    await advance(500);
    expect(savedRect("b")).toEqual({ x: 600, y: 300, w: 500, h: 200 });
    expect(docLayer("b").sizing).toBe("reflow");
    await fireEvent.click(within(inspector).getByRole("button", { name: "Lock aspect ratio" }));
    w.value = "1000";
    await fireEvent.change(w);
    await advance(500);
    expect(savedRect("b")).toEqual({ x: 600, y: 300, w: 1000, h: 400 });
  });
});

// ---- Commands -----------------------------------------------------------------------------------------

describe("editor commands", () => {
  const ids = () => registry.list.map((c) => c.id);

  it("are registered only while the editor is mounted", async () => {
    expect(ids().some((id) => id.startsWith("editor."))).toBe(false);
    const r = await mount();
    expect(ids()).toEqual(expect.arrayContaining(["editor.snap", "editor.grid", "editor.align.left", "editor.selectAll"]));
    r.unmount();
    expect(ids().some((id) => id.startsWith("editor."))).toBe(false);
  });

  it("shortcuts run through the shell's registry, never while typing", async () => {
    const { provideContext, runShortcut } = await import("$lib/commands/registry.svelte");
    // The shell provides the route: the editor's keys work only in the layout editor.
    const drop = provideContext({ route: "screenEditor" });
    onTestFinished(drop);
    const { container } = await mount();
    const grid = key({ key: "g", code: "KeyG", ctrlKey: true });
    document.body.dispatchEvent(grid);
    expect(runShortcut(grid)).toBe(true);
    expect(lastUi().screenEditor?.grid).toBe(true);

    // Typing in the screen name: no command.
    api.setUiState.mockClear();
    const name = screen.getByLabelText("Screen name");
    const typed = key({ key: "g", code: "KeyG", ctrlKey: true });
    Object.defineProperty(typed, "target", { value: name });
    expect(runShortcut(typed)).toBe(false);
    expect(api.setUiState).not.toHaveBeenCalled();

    // Ctrl+A selects every layer.
    expect(runShortcut(key({ key: "a", code: "KeyA", ctrlKey: true }))).toBe(true);
    await tick();
    expect(container.querySelectorAll(".box[data-selected]")).toHaveLength(3);
  });
});

// ---- Content sizing ---------------------------------------------------------------------------------

describe("content sizing", () => {
  it("shows absent sizing as Reflow and saves a change", async () => {
    const { container } = await mount();
    await fireEvent.click(container.querySelector('li[data-layer="b"] .main')!);
    await vi.dynamicImportSettled();
    await tick();
    const inspector = screen.getByRole("complementary", { name: "Inspector" });
    const select = within(inspector).getByLabelText("Content sizing") as HTMLSelectElement;
    expect(select.value).toBe("reflow");
    await fireEvent.change(select, { target: { value: "fit" } });
    await advance(500);
    expect(lastSaved().layers.find((l) => l.id === "b")!.sizing).toBe("fit");
    expect(lastSaved().layers.find((l) => l.id === "a")!.sizing).toBeUndefined();
  });
});

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ControlViewState,
  FlowInfo,
  LiveLayerView,
  LiveScreenView,
  Project,
  ScreenDoc,
  ServerStatus,
  SourceSummary,
  TemplateScreenLayer,
  UiState,
} from "$lib/types";
import type { SheetEditorProps } from "$lib/components/sheet/types";

// ---- Mocks --------------------------------------------------------------------------------------

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Handler>() }));
const api = vi.hoisted(() => ({
  remoteView: vi.fn(async () => ({ state: "off", on: false, takeLocked: false, crew: [], showId: null, crewUrl: null, controllerUrl: null, serverVersion: null, rttMs: null, licenseWarning: null, lastError: null, hasShowId: false })),
  cuePosition: vi.fn(async () => ({ current: null, next: null })),
  remoteDiagWatch: vi.fn(async () => null),
  licenseView: vi.fn(async () => null),
  serverStatus: vi.fn(),
  listScreens: vi.fn(),
  liveShow: vi.fn(),
  liveHide: vi.fn(),
  setLayerFields: vi.fn(),
  liveClear: vi.fn(),
  liveClearAll: vi.fn(),
  livePanic: vi.fn(),
  getScreen: vi.fn(),
  getTemplate: vi.fn(),
  setLayerVariant: vi.fn(),
  listSources: vi.fn(),
  stepSource: vi.fn(),
  listTemplates: vi.fn(),
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  on: vi.fn(),
  setTimer: vi.fn(),
  setAutoAdvance: vi.fn(),
  listFlows: vi.fn(),
  getFlow: vi.fn(),
  runFlow: vi.fn(),
  stopFlow: vi.fn(),
  setFlowArmed: vi.fn(),
  createFlow: vi.fn(),
  saveFlow: vi.fn(),
  duplicateFlow: vi.fn(),
  deleteFlow: vi.fn(),
  listRecipeInstances: vi.fn(),
  getSource: vi.fn(),
  saveSource: vi.fn(),
}));
vi.mock("$lib/api/remote", () => api);
vi.mock("$lib/api/license", () => api);
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
// The spreadsheet editor (its own chunk): a stand-in that records its props.
vi.mock("$lib/components/sheet/SheetEditor.svelte", async () => ({
  default: (await import("../fields/SheetStub.test.svelte")).default,
}));
vi.mock("$app/navigation", () => ({ goto: vi.fn(), afterNavigate: vi.fn() }));
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/control") } }));

// Observers: tests drive intersections by hand.
const obs = vi.hoisted(() => ({
  io: [] as { cb: IntersectionObserverCallback; targets: Element[]; off: boolean }[],
}));
class FakeIO {
  #rec: (typeof obs.io)[number];
  constructor(cb: IntersectionObserverCallback) {
    this.#rec = { cb, targets: [], off: false };
    obs.io.push(this.#rec);
  }
  observe(el: Element) {
    this.#rec.targets.push(el);
  }
  unobserve(el: Element) {
    this.#rec.targets = this.#rec.targets.filter((t) => t !== el);
  }
  disconnect() {
    this.#rec.off = true;
  }
  takeRecords() {
    return [];
  }
}
class FakeRO {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal("IntersectionObserver", FakeIO);
vi.stubGlobal("ResizeObserver", FakeRO);

import ControlPage from "../../../routes/(app)/control/+page.svelte";
import { loadSourcesStrip } from "./strips";
import { registerCommands, registry, runCommand, titleOf } from "$lib/commands/registry.svelte";
import { installShortcuts } from "$lib/commands/testing";
import { session } from "./variants.svelte";
import { live } from "$lib/stores/live.svelte";
import { project } from "$lib/stores/project.svelte";
import { server } from "$lib/stores/server.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { uiState } from "$lib/stores/uiState.svelte";

// ---- Fixtures -----------------------------------------------------------------------------------

const running = { state: "running", port: 7770, url: "http://127.0.0.1:7770", lanUrl: null } satisfies ServerStatus;

const current: Project = {
  path: "C:\\shows\\control",
  name: "Control Show",
  createdAt: "2026-01-02T10:00:00Z",
  settings: { port: 7770, lanAccess: false, defaultResolution: { width: 1920, height: 1080 } },
};

const control = (patch: Partial<ControlViewState> = {}): UiState => ({
  sidebarPinned: false,
  control: {
    view: "grid",
    gridColumns: 2,
    selectedScreenId: "s1",
    hiddenScreenIds: [],
    liveEdit: "onTake",
    hotkeys: {},
    automationOff: false,
    ...patch,
  },
});

function layer(id: string, patch: Partial<LiveLayerView> = {}): LiveLayerView {
  return {
    id,
    name: id === "lower" ? "Lower third" : id === "logo" ? "Logo" : id,
    onAir: false,
    fields: id === "lower" ? { name: "Grace", title: "CTO" } : {},
    fieldDefs:
      id === "lower"
        ? [
            { key: "name", label: "Name", type: "text", default: "" },
            { key: "title", label: "Title", type: "text", default: "" },
          ]
        : [],
    pending: false,
    ...patch,
  };
}

function view(id: string, layers: LiveLayerView[] = []): LiveScreenView {
  return { screen: { id, name: `Screen ${id}`, width: 1920, height: 1080, background: "transparent" }, layers };
}

const s1 = () => view("s1", [layer("logo"), layer("lower")]);

const doc: ScreenDoc = {
  id: "s1",
  name: "Screen s1",
  width: 1920,
  height: 1080,
  background: "transparent",
  updatedAt: "2026-01-01T00:00:00Z",
  layers: [
    { kind: "template", id: "logo", name: "Logo", templateId: "t2", fields: {}, vars: {}, rect: { x: 0, y: 0, w: 10, h: 10 } },
    {
      kind: "template",
      id: "lower",
      name: "Lower third",
      templateId: "t1",
      fields: { name: "Grace" },
      vars: {},
      bindings: { title: { sourceId: "guests", column: "title" } },
      rect: { x: 0, y: 0, w: 10, h: 10 },
    },
  ],
};

const guests: SourceSummary = {
  id: "guests",
  name: "Guests",
  kind: "text",
  rowCount: 3,
  index: 1,
  current: { name: "Ada", title: "CTO" },
  columns: [
    { key: "name", label: "Name" },
    { key: "title", label: "Title" },
  ],
  usedBy: ["s1"],
  auto: null,
  nextAt: null,
  nowId: null,
  nextId: null,
  updatedAt: "2026-01-01T00:00:00Z",
};

const flush = () => new Promise((r) => setTimeout(r, 0));

async function intersectAll() {
  for (const rec of obs.io) {
    if (rec.off || rec.targets.length === 0) continue;
    rec.cb(
      rec.targets.map((target) => ({ target, isIntersecting: true })) as unknown as IntersectionObserverEntry[],
      {} as IntersectionObserver,
    );
  }
  await tick();
}

async function mount(screens: LiveScreenView[], ui: UiState = control()) {
  api.listScreens.mockResolvedValue(screens);
  uiState.set(ui);
  const r = render(ControlPage);
  await flush();
  await flush();
  // The sources strip is a lazy chunk.
  await loadSourcesStrip();
  // The rows' expanded area (with the variant support) is a chunk of its own.
  await import("./LayerBody.svelte");
  // The hotkey commands are a chunk of their own.
  await import("./commands.svelte");
  await flush();
  await intersectAll();
  return r;
}

const frames = () => [...document.querySelectorAll("iframe")];
const lastUi = (): ControlViewState => (api.setUiState.mock.calls.at(-1)![0] as UiState).control;
const row = (id: string) => document.querySelector<HTMLElement>(`[data-layer="${id}"]`)!;

beforeEach(() => {
  vi.clearAllMocks();
  obs.io.length = 0;
  events.handlers.clear();
  server.status = null;
  live.screens = new Map();
  api.on.mockImplementation((name: string, h: Handler) => {
    events.handlers.set(name, h);
    return () => events.handlers.delete(name);
  });
  api.serverStatus.mockResolvedValue(running);
  api.getScreen.mockImplementation(async (id: string) => ({ doc: { ...doc, id }, live: s1(), warnings: [] }));
  api.listSources.mockResolvedValue([guests]);
  api.listTemplates.mockResolvedValue([
    { id: "t1", name: "Classic bar", description: "", preset: null, fieldCount: 2, variableCount: 0, updatedAt: "" },
  ]);
  api.setUiState.mockImplementation(async (v: UiState) => v);
  api.liveShow.mockImplementation(async (_s: string, l: string) => layer(l, { onAir: true }));
  api.liveHide.mockImplementation(async (_s: string, l: string) => layer(l, { onAir: false }));
  api.setLayerFields.mockImplementation(async (_s: string, l: string, f: Record<string, string>) =>
    layer(l, { fields: { ...layer(l).fields, ...f } }),
  );
  api.livePanic.mockResolvedValue([s1()]);
  api.liveClear.mockImplementation(async (id: string) => view(id, [layer("logo"), layer("lower")]));
  api.liveClearAll.mockResolvedValue([s1()]);
  api.stepSource.mockImplementation(async (_id: string, step: { delta: number }) => ({
    ...guests,
    index: guests.index + step.delta,
  }));
  api.listFlows.mockResolvedValue([]);
  api.runFlow.mockResolvedValue(undefined);
  api.stopFlow.mockResolvedValue(undefined);
  api.listRecipeInstances.mockResolvedValue([]);
  project.current = current;
  // What the shell does: hotkeys are commands run by its capture-phase keydown.
  offKeys = installShortcuts({ route: "control" });
});

let offKeys = () => {};

afterEach(() => {
  offKeys();
  cleanup();
  toasts.clear();
  vi.useRealTimers();
});

// ---- Layout -------------------------------------------------------------------------------------

describe("Control layout", () => {
  it("no screens: offers to create the first one", async () => {
    await mount([]);
    expect(screen.getByRole("heading", { name: "Create your first screen" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "New screen" })).toBeTruthy();
  });

  it("all screens hidden: Show screens clears the hidden list", async () => {
    await mount([s1()], control({ hiddenScreenIds: ["s1"] }));
    expect(frames()).toHaveLength(0);
    await fireEvent.click(screen.getByRole("button", { name: "Show screens" }));
    expect(lastUi().hiddenScreenIds).toEqual([]);
  });

  it("grid: at most 6 live iframes, the selected screen always among them", async () => {
    const all = ["a", "b", "c", "d", "e", "f", "g", "h"].map((id) => view(id));
    await mount(all, control({ selectedScreenId: "h" }));
    expect(frames()).toHaveLength(6);
    expect(frames().some((f) => f.src.endsWith("/screen/h?mute=1"))).toBe(true);
    expect(document.querySelectorAll("[data-screen]").length).toBe(8);
  });

  it("a hidden screen renders no tile and no iframe", async () => {
    await mount([s1(), view("s2")], control({ hiddenScreenIds: ["s2"] }));
    expect(frames().map((f) => f.src)).toEqual(["http://127.0.0.1:7770/screen/s1?mute=1"]);
    expect(document.querySelector('[data-screen="s2"]')).toBeNull();
  });

  it("single view: exactly one iframe, of the selected screen", async () => {
    await mount([s1(), view("s2"), view("s3")], control({ view: "single", selectedScreenId: "s2" }));
    expect(frames()).toHaveLength(1);
    expect(frames()[0].src).toBe("http://127.0.0.1:7770/screen/s2?mute=1");
    const list = screen.getByRole("listbox", { name: "Screens" });
    const opt = within(list).getByRole("option", { selected: true });
    expect(opt.textContent).toContain("Screen s2");
    await fireEvent.keyDown(opt, { key: "ArrowDown" });
    expect(lastUi().selectedScreenId).toBe("s3");
  });

  it("toolbar choices write the ui-state", async () => {
    await mount([s1(), view("s2")]);
    await fireEvent.click(screen.getByRole("radio", { name: "Single" }));
    expect(lastUi().view).toBe("single");
    uiState.set(control());
    await tick();

    const cols = screen.getByLabelText("Columns") as HTMLInputElement;
    cols.value = "4";
    await fireEvent.change(cols);
    expect(lastUi().gridColumns).toBe(4);

    await fireEvent.click(screen.getByRole("radio", { name: "Immediate" }));
    expect(lastUi().liveEdit).toBe("immediate");

    await fireEvent.click(screen.getByRole("button", { name: "Screens" }));
    await fireEvent.click(screen.getByRole("menuitemcheckbox", { name: /Screen s2/ }));
    expect(lastUi().hiddenScreenIds).toEqual(["s2"]);
  });

  it("clicking a tile selects its screen", async () => {
    await mount([s1(), view("s2")]);
    await fireEvent.click(screen.getByRole("button", { name: "Screen s2" }));
    expect(lastUi().selectedScreenId).toBe("s2");
  });
});

// ---- Layers -------------------------------------------------------------------------------------

describe("Control layers", () => {
  it("lists layers front first with template names and bound sources", async () => {
    await mount([s1()]);
    const rows = [...document.querySelectorAll("[data-layer]")].map((el) => el.getAttribute("data-layer"));
    expect(rows).toEqual(["lower", "logo"]);
    expect(row("lower").textContent).toContain("Classic bar");
    const strip = document.querySelector('[data-source="guests"]')!;
    expect(strip.textContent).toContain("Ada · CTO");
    expect(strip.textContent).toContain("2 / 3");
  });

  it("TAKE is optimistic and calls liveShow; CLEAR calls liveHide", async () => {
    let release!: (v: LiveLayerView) => void;
    api.liveShow.mockReturnValueOnce(new Promise((r) => (release = r)));
    await mount([s1()]);
    await fireEvent.click(screen.getByRole("button", { name: "Take Lower third" }));
    expect(within(row("lower")).getByRole("img", { name: "On air" })).toBeTruthy();
    expect(api.liveShow).toHaveBeenCalledWith("s1", "lower");
    release(layer("lower", { onAir: true }));
    await flush();
    await fireEvent.click(screen.getByRole("button", { name: "Clear Lower third" }));
    expect(api.liveHide).toHaveBeenCalledWith("s1", "lower");
    expect(within(row("lower")).getByRole("img", { name: "Off air" })).toBeTruthy();
  });

  it("a failed TAKE rolls back and toasts", async () => {
    api.liveShow.mockRejectedValueOnce({ code: "notFound", message: "gone" });
    await mount([s1()]);
    await fireEvent.click(screen.getByRole("button", { name: "Take Lower third" }));
    await flush();
    expect(within(row("lower")).getByRole("img", { name: "Off air" })).toBeTruthy();
    expect(toasts.list.at(-1)?.message).toContain("gone");
  });

  it("field edits are debounced into setLayerFields and flushed before TAKE", async () => {
    await mount([s1()]);
    await fireEvent.click(screen.getByRole("button", { name: "Fields and hotkey of Lower third" }));
    const input = within(row("lower")).getByLabelText("Name") as HTMLInputElement;
    expect(input.value).toBe("Grace");

    vi.useFakeTimers();
    await fireEvent.focus(input);
    await fireEvent.input(input, { target: { value: "Ad" } });
    await fireEvent.input(input, { target: { value: "Ada" } });
    await vi.advanceTimersByTimeAsync(149);
    expect(api.setLayerFields).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(api.setLayerFields).toHaveBeenCalledExactlyOnceWith("s1", "lower", { name: "Ada" });

    // A live update with another value never overwrites the focused field.
    events.handlers.get("live-changed")?.(view("s1", [layer("logo"), layer("lower", { fields: { name: "Old", title: "CTO" } })]));
    await tick();
    expect(input.value).toBe("Ada");

    // Typed, then TAKE before the debounce: the edit goes out first.
    await fireEvent.input(input, { target: { value: "Ada L" } });
    await fireEvent.click(screen.getByRole("button", { name: "Take Lower third" }));
    await vi.advanceTimersByTimeAsync(0);
    expect(api.setLayerFields).toHaveBeenLastCalledWith("s1", "lower", { name: "Ada L" });
    const fieldsOrder = api.setLayerFields.mock.invocationCallOrder.at(-1)!;
    expect(api.liveShow.mock.invocationCallOrder[0]).toBeGreaterThan(fieldsOrder);
  });

  it("a bound field is read-only with its source, and Prev/Next step the source", async () => {
    await mount([s1()]);
    await fireEvent.click(screen.getByRole("button", { name: "Fields and hotkey of Lower third" }));
    const r = row("lower");
    // Read-only: no editable input for the bound field.
    const title = within(r).queryByLabelText("Title");
    expect(title instanceof HTMLInputElement || title instanceof HTMLTextAreaElement).toBe(false);
    expect(r.textContent).toContain("Guests · Title");
    const [prev] = within(r).getAllByRole("button", { name: "Previous row of Guests" });
    await fireEvent.click(prev);
    expect(api.stepSource).toHaveBeenCalledWith("guests", { to: "by", delta: -1 });
    const strip = document.querySelector<HTMLElement>('[data-source="guests"]')!;
    await fireEvent.click(within(strip).getByRole("button", { name: "Next row of Guests" }));
    expect(api.stepSource).toHaveBeenLastCalledWith("guests", { to: "by", delta: 1 });
  });

  it("shows Changes on next take while pending", async () => {
    await mount([view("s1", [layer("lower", { onAir: true, pending: true })])]);
    expect(row("lower").textContent).toContain("Changes on next take");
  });

  it("a screen without layers links to the layout editor", async () => {
    await mount([view("s1")]);
    const link = screen.getByRole("link", { name: "Open layout editor" });
    expect(link.getAttribute("href")).toBe("/screens/s1");
  });

  it("Clear screen clears the selected screen; Clear every screen clears all", async () => {
    await mount([view("s1", [layer("lower", { onAir: true })])]);
    // The toolbar button (the tile has the same action).
    const clear = screen.getAllByRole("button", { name: "Clear Screen s1" })[0];
    expect(clear.textContent).toContain("Clear screen");
    await fireEvent.click(clear);
    expect(api.liveClear).toHaveBeenCalledWith("s1");
    live.put(view("s1", [layer("lower", { onAir: true })]));
    await tick();
    await fireEvent.click(screen.getByRole("button", { name: "More clear options" }));
    await fireEvent.click(screen.getByRole("menuitem", { name: "Clear every screen" }));
    expect(api.liveClearAll).toHaveBeenCalled();
  });
});

// ---- Hotkeys ------------------------------------------------------------------------------------

describe("Control hotkeys", () => {
  it("a mapped F-key toggles its layer and stops the browser default", async () => {
    await mount([s1()], control({ hotkeys: { "s1/lower": "F5" } }));
    expect(row("lower").textContent).toContain("F5");
    const notCancelled = await fireEvent.keyDown(window, { key: "F5" });
    expect(notCancelled).toBe(false);
    expect(api.liveShow).toHaveBeenCalledWith("s1", "lower");
    await flush();
    await fireEvent.keyDown(window, { key: "F5" });
    expect(api.liveHide).toHaveBeenCalledWith("s1", "lower");
  });

  it("ignored while typing and on repeat; unmapped keys pass through", async () => {
    await mount([s1()], control({ hotkeys: { "s1/lower": "F1" } }));
    await fireEvent.keyDown(window, { key: "F1", repeat: true });
    expect(await fireEvent.keyDown(window, { key: "F2" })).toBe(true);
    await fireEvent.click(screen.getByRole("button", { name: "Fields and hotkey of Lower third" }));
    const input = within(row("lower")).getByLabelText("Name");
    await fireEvent.keyDown(input, { key: "F1" });
    expect(api.liveShow).not.toHaveBeenCalled();
  });

  it("Shift+Esc is PANIC, no confirm, and announces it", async () => {
    await mount([view("s1", [layer("lower", { onAir: true })])]);
    expect(await fireEvent.keyDown(window, { key: "Escape", shiftKey: true })).toBe(false);
    expect(api.livePanic).toHaveBeenCalledOnce();
    expect(within(row("lower")).getByRole("img", { name: "Off air" })).toBeTruthy();
    await flush();
    expect(toasts.list.at(-1)?.message).toContain("PANIC");
    const panic = screen.getByRole("button", { name: /Panic/ });
    // An emergency control: the danger button, filled (the toolbar's .panic rule).
    expect(panic.classList.contains("danger")).toBe(true);
    expect(panic.classList.contains("panic")).toBe(true);
    await fireEvent.click(panic);
    expect(api.livePanic).toHaveBeenCalledTimes(2);
  });

  it("Set hotkey captures the next F-key (the previous owner loses it); Esc cancels", async () => {
    await mount([s1()], control({ hotkeys: { "s1/logo": "Shift+F2" } }));
    await fireEvent.click(screen.getByRole("button", { name: "Fields and hotkey of Lower third" }));
    await fireEvent.click(screen.getByRole("button", { name: "Set hotkey for Lower third" }));
    expect(row("lower").textContent).toContain("Esc cancels");
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(api.setUiState).not.toHaveBeenCalled();
    expect(row("lower").textContent).not.toContain("Esc cancels");

    await fireEvent.click(screen.getByRole("button", { name: "Set hotkey for Lower third" }));
    await fireEvent.keyDown(window, { key: "F2", shiftKey: true });
    expect(lastUi().hotkeys).toEqual({ "s1/lower": "Shift+F2" });
    // Capturing never toggles a layer.
    expect(api.liveShow).not.toHaveBeenCalled();
    await tick();
    await fireEvent.click(screen.getByRole("button", { name: "Remove hotkey of Lower third" }));
    expect(lastUi().hotkeys).toEqual({});
  });

  it("a hotkey shared with another command warns (it runs nothing) and links to Keyboard shortcuts", async () => {
    const off = registerCommands([{ id: "test.global", title: "Toggle theme", section: "settings", defaultKeys: ["F5"], run: vi.fn() }]);
    try {
      await mount([s1()], control({ hotkeys: { "s1/lower": "F5" } }));
      await fireEvent.click(screen.getByRole("button", { name: "Fields and hotkey of Lower third" }));
      const alert = await within(row("lower")).findByRole("alert");
      expect(alert.textContent).toContain("F5 runs nothing: it is also assigned to Toggle theme.");
      expect(within(alert).getByRole("link", { name: "Change one in Keyboard shortcuts" }).getAttribute("href")).toBe("/settings#shortcuts");
    } finally {
      off();
    }
  });
});

// ---- Automation ----------------------------------------------------------------------------

describe("Control timers", () => {
  it("a timer field's Start goes straight to setTimer and the reply updates the layer", async () => {
    const state = JSON.stringify({ run: false, at: 0, base: 60_000, dir: "down", end: "stop" });
    const clock = (patch: Partial<LiveLayerView> = {}) =>
      layer("clock", {
        name: "Clock",
        fields: { game: state },
        fieldDefs: [{ key: "game", label: "Game clock", type: "timer", default: state }],
        ...patch,
      });
    api.setTimer.mockImplementation(async () =>
      clock({ fields: { game: JSON.stringify({ run: true, at: 1, base: 60_000, dir: "down", end: "stop" }) } }),
    );
    await mount([view("s1", [clock()])]);
    await fireEvent.click(screen.getByRole("button", { name: "Fields and hotkey of Clock" }));
    await fireEvent.click(await screen.findByRole("button", { name: "Start Game clock" }));
    expect(api.setTimer).toHaveBeenCalledExactlyOnceWith("s1", "clock", "game", { action: "start" });
    await flush();
    expect(JSON.parse(live.screens.get("s1")!.layers[0].fields.game).run).toBe(true);
    expect(api.setLayerFields).not.toHaveBeenCalled();
  });
});

describe("Control auto-advance", () => {
  const AUTO = { everyMs: 5000, loop: true, transition: "update" } as const;
  const strip = () => document.querySelector<HTMLElement>('[data-source="guests"]')!;

  it("Auto turns auto-advance on with the defaults and off again", async () => {
    api.setAutoAdvance.mockImplementation(async (_id: string, auto: unknown) => ({ ...guests, auto, nextAt: null }));
    await mount([s1()]);
    const sw = within(strip()).getByRole("button", { name: "Auto-advance Guests" });
    expect(sw.getAttribute("aria-pressed")).toBe("false");
    await fireEvent.click(sw);
    expect(api.setAutoAdvance).toHaveBeenLastCalledWith("guests", AUTO);
    await flush();
    expect(sw.getAttribute("aria-pressed")).toBe("true");
    expect(strip().textContent).toContain("5 s");
    await fireEvent.click(sw);
    expect(api.setAutoAdvance).toHaveBeenLastCalledWith("guests", null);
  });

  it("a failed toggle rolls back and toasts", async () => {
    api.setAutoAdvance.mockRejectedValueOnce({ code: "invalid", message: "nope" });
    await mount([s1()]);
    const sw = within(strip()).getByRole("button", { name: "Auto-advance Guests" });
    await fireEvent.click(sw);
    await flush();
    expect(sw.getAttribute("aria-pressed")).toBe("false");
    expect(toasts.list.at(-1)?.message).toContain("nope");
  });

  it("settings (lazy popover) are remembered while off and applied at once while on", async () => {
    api.setAutoAdvance.mockImplementation(async (_id: string, auto: unknown) => ({ ...guests, auto, nextAt: null }));
    await mount([s1()]);
    await fireEvent.click(within(strip()).getByRole("button", { name: "Auto-advance settings of Guests" }));
    const pop = await screen.findByRole("dialog", { name: /Auto-advance/ });
    const every = within(pop).getByLabelText("Every") as HTMLInputElement;
    every.value = "12";
    await fireEvent.change(every);
    await fireEvent.click(within(pop).getByRole("radio", { name: "Play out & in" }));
    expect(api.setAutoAdvance).not.toHaveBeenCalled();
    expect(pop.textContent).toContain("Used when Auto is on");

    await fireEvent.click(within(strip()).getByRole("button", { name: "Auto-advance Guests" }));
    expect(api.setAutoAdvance).toHaveBeenLastCalledWith("guests", { everyMs: 12_000, loop: true, transition: "retake" });
    await flush();
    await fireEvent.click(within(pop).getByRole("switch", { name: "Loop" }));
    expect(api.setAutoAdvance).toHaveBeenLastCalledWith("guests", { everyMs: 12_000, loop: false, transition: "retake" });

    await fireEvent.keyDown(pop, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: /Auto-advance/ })).toBeNull();
  });

  it("while armed a CSS countdown runs for the remaining time, restarted by source-changed", async () => {
    const now = vi.spyOn(Date, "now").mockReturnValue(1_000_000);
    api.listSources.mockResolvedValue([{ ...guests, auto: AUTO, nextAt: 1_004_000 }]);
    await mount([s1()]);
    let bar = strip().querySelector<HTMLElement>("[data-countdown]")!;
    expect(bar.style.animationDuration).toBe("4000ms");

    events.handlers.get("source-changed")?.({
      ...guests,
      index: 2,
      current: { name: "Linus", title: "Kernel" },
      auto: AUTO,
      nextAt: 1_005_000,
    });
    await tick();
    bar = strip().querySelector<HTMLElement>("[data-countdown]")!;
    expect(bar.style.animationDuration).toBe("5000ms");
    expect(strip().textContent).toContain("Linus · Kernel");
    expect(strip().textContent).toContain("3 / 3");

    // Disarmed (no bound layer on air): no bar.
    events.handlers.get("source-changed")?.({ ...guests, auto: AUTO, nextAt: null });
    await tick();
    expect(strip().querySelector("[data-countdown]")).toBeNull();
    now.mockRestore();
  });
});

function cueFlow(id: string, name: string, patch: Partial<FlowInfo> = {}, label: string | null = null): FlowInfo {
  return {
    id,
    name,
    armed: true,
    retrigger: "restart",
    origin: null,
    nodes: [
      { id: `${id}-t`, type: "trigger.cue", config: { label }, position: { x: 0, y: 0 } },
      { id: `${id}-a`, type: "action.take", config: { screenId: "s1", layerId: "lower", forMs: null }, position: { x: 240, y: 0 } },
    ],
    edges: [{ id: `${id}-e`, from: { node: `${id}-t`, port: "fired" }, to: { node: `${id}-a`, port: "in" }, kind: "event" }],
    lastFiredAt: null,
    updatedAt: "2026-10-04T08:00:00Z",
    warnings: [],
    running: false,
    readOnly: false,
    ...patch,
  };
}

describe("Control cues", () => {
  const open = cueFlow("f1", "Open show");
  const cueRow = (id = "f1") => document.querySelector<HTMLElement>(`[data-cue="${id}"]`)!;
  const cues = () => screen.getByRole("region", { name: "Cues" });

  it("one button per cue flow, labelled by the cue label; rules are no cues", async () => {
    const rule: FlowInfo = {
      ...cueFlow("r1", "Strap on NOW"),
      nodes: [{ id: "t", type: "trigger.layer", config: { screenId: "s1", layerId: "lower", onAir: true }, position: { x: 0, y: 0 } }],
      edges: [],
    };
    api.listFlows.mockResolvedValue([open, cueFlow("f2", "Second", {}, "Guest in"), rule]);
    await mount([s1()]);
    expect(within(cues()).getByRole("button", { name: "Run Open show" })).toBeTruthy();
    expect(within(cues()).getByRole("button", { name: "Run Guest in" })).toBeTruthy();
    expect(within(cues()).queryByText("Strap on NOW")).toBeNull();
  });

  it("Run shows running at once, follows flow-state (last fired, failures), Stop stops", async () => {
    api.listFlows.mockResolvedValue([open]);
    await mount([s1()]);
    await fireEvent.click(screen.getByRole("button", { name: "Run Open show" }));
    expect(api.runFlow).toHaveBeenCalledWith("f1");
    expect(cueRow().textContent).toContain("Running");

    const at = new Date(Date.now() - 125_000).toISOString();
    events.handlers.get("flow-state")?.({ id: "f1", state: "running", node: "f1-a", message: null, at });
    await tick();
    await fireEvent.click(screen.getByRole("button", { name: "Stop Open show" }));
    expect(api.stopFlow).toHaveBeenCalledWith("f1");

    events.handlers.get("flow-state")?.({ id: "f1", state: "stopped", node: null, message: null, at });
    await tick();
    expect(cueRow().textContent).not.toContain("Running");
    expect(cueRow().textContent).toContain("2 min ago");
    expect(screen.getByRole("button", { name: "Run Open show" })).toBeTruthy();

    events.handlers.get("flow-state")?.({ id: "f1", state: "failed", node: "f1-a", message: "Layer gone", at });
    await tick();
    expect(cueRow().textContent).toContain("Failed: Layer gone");
  });

  it("a failed run toasts and drops the running state", async () => {
    api.listFlows.mockResolvedValue([open]);
    api.runFlow.mockRejectedValueOnce({ code: "notFound", message: "no such flow" });
    await mount([s1()]);
    await fireEvent.click(screen.getByRole("button", { name: "Run Open show" }));
    await flush();
    expect(cueRow().textContent).not.toContain("Running");
    expect(toasts.list.at(-1)?.message).toContain("no such flow");
  });

  it("a disarmed cue is disabled with the text Disarmed; its hotkey doesn't run it", async () => {
    api.listFlows.mockResolvedValue([cueFlow("f1", "Open show", { armed: false })]);
    await mount([s1()], control({ hotkeys: { "flow/f1": "F6" } }));
    expect(cueRow().textContent).toContain("Disarmed");
    expect((screen.getByRole("button", { name: "Run Open show" }) as HTMLButtonElement).disabled).toBe(true);
    await fireEvent.keyDown(window, { key: "F6" });
    expect(api.runFlow).not.toHaveBeenCalled();
    expect(toasts.list.at(-1)?.message).toBe("Open show is disarmed.");
  });

  it("a cue hotkey runs it (again = run again); legacy macro/ keys too; Set hotkey writes flow/<id>", async () => {
    api.listFlows.mockResolvedValue([open, cueFlow("m1", "Migrated")]);
    await mount([s1()], control({ hotkeys: { "flow/f1": "F6", "macro/m1": "Shift+F6", "s1/lower": "F7" } }));
    expect(cueRow().querySelector(".badge")?.textContent).toBe("F6");
    expect(cueRow("m1").querySelector(".badge")?.textContent).toBe("Shift+F6");
    expect(await fireEvent.keyDown(window, { key: "F6" })).toBe(false);
    await fireEvent.keyDown(window, { key: "F6" });
    expect(api.runFlow).toHaveBeenCalledTimes(2);
    await fireEvent.keyDown(window, { key: "F6", shiftKey: true });
    expect(api.runFlow).toHaveBeenLastCalledWith("m1");
    expect(api.liveShow).not.toHaveBeenCalled();
    // Layer keys keep working.
    await fireEvent.keyDown(window, { key: "F7" });
    expect(api.liveShow).toHaveBeenCalledWith("s1", "lower");

    // Re-assigning the migrated cue replaces its legacy key.
    await fireEvent.click(screen.getByRole("button", { name: "Set hotkey for Migrated" }));
    expect(cueRow("m1").textContent).toContain("Esc cancels");
    await fireEvent.keyDown(window, { key: "F8" });
    expect(lastUi().hotkeys).toEqual({ "flow/f1": "F6", "s1/lower": "F7", "flow/m1": "F8" });
  });

  it("no cues: New cue opens the linear editor (lazy)", async () => {
    await mount([s1()]);
    await fireEvent.click(within(cues()).getByRole("button", { name: "New cue" }));
    expect(await screen.findByRole("dialog", { name: "New cue" })).toBeTruthy();
  });

  it("a flow file changed on disk reloads that flow; a removed one goes", async () => {
    api.listFlows.mockResolvedValue([open]);
    await mount([s1()]);
    api.getFlow.mockResolvedValue(cueFlow("f1", "Open show", {}, "Opening"));
    events.handlers.get("project-files-changed")?.({ kind: "flow", id: "f1", change: "changed", message: null });
    await flush();
    expect(api.getFlow).toHaveBeenCalledWith("f1");
    expect(screen.getByRole("button", { name: "Run Opening" })).toBeTruthy();
    events.handlers.get("project-files-changed")?.({ kind: "flow", id: "f1", change: "removed", message: null });
    await flush();
    expect(cueRow()).toBeNull();
  });
});

describe("Control cues, scoped to the selected screen", () => {
  const other: FlowInfo = {
    ...cueFlow("f9", "Other screen cue"),
    nodes: [
      { id: "t", type: "trigger.cue", config: { label: null }, position: { x: 0, y: 0 } },
      { id: "a", type: "action.clearScreen", config: { screenId: "s2" }, position: { x: 240, y: 0 } },
    ],
  };
  const cues = () => screen.getByRole("region", { name: "Cues" });

  it("a cue aimed at another screen is hidden until Show all cues", async () => {
    api.listFlows.mockResolvedValue([cueFlow("f1", "Open show"), other]);
    await mount([s1(), view("s2", [])]);
    expect(within(cues()).getByRole("button", { name: "Run Open show" })).toBeTruthy();
    expect(within(cues()).queryByRole("button", { name: "Run Other screen cue" })).toBeNull();
    const all = within(cues()).getByRole("button", { name: "Show all cues" });
    expect(all.getAttribute("aria-pressed")).toBe("false");
    await fireEvent.click(all);
    expect(all.getAttribute("aria-pressed")).toBe("true");
    expect(within(cues()).getByRole("button", { name: "Run Other screen cue" })).toBeTruthy();
    await fireEvent.click(all);
    expect(within(cues()).queryByRole("button", { name: "Run Other screen cue" })).toBeNull();
  });

  it("none for this screen: says so and offers New cue and Show all cues", async () => {
    api.listFlows.mockResolvedValue([other]);
    await mount([s1(), view("s2", [])]);
    expect(within(cues()).getByText("No cues for this screen.")).toBeTruthy();
    expect(within(cues()).getAllByRole("button", { name: "New cue" }).length).toBe(2);
    const show = within(cues()).getAllByRole("button", { name: "Show all cues" });
    await fireEvent.click(show.at(-1)!);
    expect(within(cues()).getByRole("button", { name: "Run Other screen cue" })).toBeTruthy();
    // Back off for the next tests (session state).
    await fireEvent.click(within(cues()).getByRole("button", { name: "Show all cues" }));
  });
});

describe("Automation off", () => {
  it("the switch next to PANIC writes ui-state and shows the amber state with text", async () => {
    await mount([s1()]);
    const sw = screen.getByRole("switch", { name: "Automation off" });
    expect(sw.getAttribute("aria-checked")).toBe("false");
    expect(sw.closest(".auto")?.classList.contains("on")).toBe(false);
    await fireEvent.click(sw);
    expect(lastUi().automationOff).toBe(true);
    await tick();
    expect(sw.getAttribute("aria-checked")).toBe("true");
    expect(sw.closest(".auto")?.classList.contains("on")).toBe(true);
    expect(sw.closest(".auto")?.textContent).toContain("Automation off");
    await fireEvent.click(sw);
    expect(lastUi().automationOff).toBe(false);
  });

  it("a failed write rolls back and toasts", async () => {
    api.setUiState.mockRejectedValueOnce(new Error("disk full"));
    await mount([s1()]);
    const sw = screen.getByRole("switch", { name: "Automation off" });
    await fireEvent.click(sw);
    await flush();
    expect(sw.getAttribute("aria-checked")).toBe("false");
    expect(toasts.list.at(-1)?.message).toBe("Couldn't switch automation off: disk full");
  });
});

describe("Control automation", () => {
  const side = () => screen.getByRole("complementary", { name: "Selected screen" });

  it("sections in order: layers, cues, automation (lazy); no flows list; Manage in Flows at the bottom", async () => {
    api.listRecipeInstances.mockResolvedValue([]);
    await mount([s1()]);
    const strip = screen.getByRole("region", { name: "Automation" });
    await waitFor(() => expect(within(strip).getByText("Nothing runs on its own here.")).toBeTruthy());
    expect(within(strip).getAllByRole("button", { name: "Add automation…" }).length).toBe(2);
    expect(api.listRecipeInstances).toHaveBeenCalledTimes(1);
    const order = [...side().querySelectorAll("section")].map((s) => s.getAttribute("aria-label") ?? s.querySelector("h2")?.textContent);
    expect(order.indexOf("Cues")).toBeGreaterThan(0);
    expect(order.indexOf("Cues")).toBeLessThan(order.indexOf("Automation"));
    expect(screen.queryByRole("region", { name: "Flows" })).toBeNull();
    const link = within(side()).getByRole("link", { name: "Manage in Flows →" });
    expect(link.getAttribute("href")).toBe("/flows");
    expect(side().lastElementChild).toBe(link);
  });

  it("operator words only: no flow or recipe in the panel's visible text", async () => {
    const rule: FlowInfo = {
      ...cueFlow("r1", "Strap on air"),
      nodes: [{ id: "t", type: "trigger.layer", config: { screenId: "s1", layerId: "lower", onAir: true }, position: { x: 0, y: 0 } }],
      edges: [],
    };
    api.listFlows.mockResolvedValue([cueFlow("f1", "Open show"), rule]);
    await mount([s1()]);
    await waitFor(() => expect(within(side()).getByText("Other rules")).toBeTruthy());
    const text = (side().textContent ?? "").replace("Manage in Flows →", "");
    expect(text).not.toMatch(/flow|recipe/i);
  });

  it("schedule sources stay out of the sources strip (their automation panel runs them)", async () => {
    api.listSources.mockResolvedValue([guests, { ...guests, id: "sched", name: "Run of show", kind: "schedule" }]);
    await mount([s1()]);
    expect(document.querySelector('[data-source="guests"]')).toBeTruthy();
    expect(document.querySelector('[data-source="sched"]')).toBeNull();
  });
});

describe("Control variants", () => {
  const look = (id: string, name: string) => ({ id, name, vars: {}, html: "<div></div>", css: "" });

  beforeEach(() => {
    session.transition = "crossfade";
    api.listTemplates.mockResolvedValue([
      { id: "t1", name: "Classic bar", description: "", preset: null, fieldCount: 2, variableCount: 0, variantCount: 3, updatedAt: "u1" },
      { id: "t2", name: "Logo bug", description: "", preset: null, fieldCount: 0, variableCount: 0, variantCount: 1, updatedAt: "u1" },
    ]);
    api.getTemplate.mockImplementation(async (id: string) => ({
      id,
      name: "Classic bar",
      description: "",
      preset: null,
      fields: [],
      variables: [],
      autoOutMs: null,
      updatedAt: "u1",
      defaultVariant: "full",
      variants: [look("full", "Full"), look("compact", "Compact"), look("split", "Split")],
    }));
    api.setLayerVariant.mockImplementation(async (_s: string, l: string, variant: string) => ({
      doc,
      live: view("s1", [layer("logo", { variant: "default" }), layer(l, { variant, onAir: true, pending: true })]),
      warnings: [],
    }));
  });

  const s1v = () => view("s1", [layer("logo", { variant: "default" }), layer("lower", { variant: "full", onAir: true })]);
  const switchTitles = () => registry.list.map(titleOf).filter((t) => t.startsWith("Switch "));

  async function expandLower() {
    await fireEvent.click(screen.getByRole("button", { name: "Fields and hotkey of Lower third" }));
    return (await vi.waitFor(() => within(row("lower")).getByLabelText("Variant of Lower third"))) as HTMLSelectElement;
  }

  it("the expanded row lists the variants (default marked) with the transition, Crossfade first", async () => {
    await mount([s1v()]);
    const select = await expandLower();
    await vi.waitFor(() => expect(select.disabled).toBe(false));
    expect([...select.options].map((o) => o.textContent)).toEqual(["Full (default)", "Compact", "Split"]);
    expect(select.value).toBe("full");
    const crossfade = within(row("lower")).getByRole("radio", { name: "Crossfade" });
    expect(crossfade.getAttribute("aria-checked")).toBe("true");
    // The eager row has no variant select; a single-variant template has none when expanded.
    await fireEvent.click(screen.getByRole("button", { name: "Fields and hotkey of Logo" }));
    expect(within(row("logo")).queryByLabelText("Variant of Logo")).toBeNull();
  });

  it("a change calls setLayerVariant with the chosen transition; staged under onTake", async () => {
    await mount([s1v()]);
    const select = await expandLower();
    await vi.waitFor(() => expect(select.disabled).toBe(false));
    await fireEvent.click(within(row("lower")).getByRole("radio", { name: "Out and in" }));
    await fireEvent.change(select, { target: { value: "compact" } });
    await flush();
    expect(api.setLayerVariant).toHaveBeenCalledExactlyOnceWith("s1", "lower", "compact", "outIn");
    expect(select.value).toBe("compact");
    expect(row("lower").textContent).toContain("Changes on next take");
  });

  it("the transition is remembered for the session", async () => {
    const r = await mount([s1v()]);
    await expandLower();
    await fireEvent.click(within(row("lower")).getByRole("radio", { name: "Out and in" }));
    r.unmount();
    await mount([s1v()]);
    await expandLower();
    expect(within(row("lower")).getByRole("radio", { name: "Out and in" }).getAttribute("aria-checked")).toBe("true");
  });

  it("a failed switch puts the select back and toasts", async () => {
    api.setLayerVariant.mockRejectedValueOnce({ code: "notFound", message: "gone" });
    await mount([s1v()]);
    const select = await expandLower();
    await vi.waitFor(() => expect(select.disabled).toBe(false));
    await fireEvent.change(select, { target: { value: "split" } });
    await flush();
    expect(select.value).toBe("full");
    expect(toasts.list.at(-1)?.message).toBe("Couldn't switch Lower third: gone");
  });

  it("registers Switch <layer> to <variant> for the selected screen; running one uses the session's transition", async () => {
    const r = await mount([s1v(), view("s2", [layer("other")])]);
    await vi.waitFor(() => expect(switchTitles()).toEqual(["Switch Lower third to Compact", "Switch Lower third to Split"]));
    session.transition = "outIn";
    await runCommand(registry.list.find((c) => titleOf(c) === "Switch Lower third to Split")!.id);
    expect(api.setLayerVariant).toHaveBeenCalledWith("s1", "lower", "split", "outIn");
    await vi.waitFor(() => expect(switchTitles()).toEqual(["Switch Lower third to Full", "Switch Lower third to Compact"]));
    // Another screen without variants: none.
    uiState.set(control({ selectedScreenId: "s2" }));
    await vi.waitFor(() => expect(switchTitles()).toEqual([]));
    uiState.set(control({ selectedScreenId: "s1" }));
    await vi.waitFor(() => expect(switchTitles()).toHaveLength(2));
    r.unmount();
    expect(switchTitles()).toEqual([]);
  });
});


// ---- Data editing: list summaries + drawer, resizable panel ---------------------------------------

describe("Control list fields", () => {
  const COLS = [
    { key: "name", label: "Name" },
    { key: "title", label: "Title" },
  ];
  const boardDef = { key: "rows", label: "Rows", type: "list" as const, default: "[]", columns: COLS };
  const board = (patch: Partial<LiveLayerView> = {}) =>
    layer("board", { name: "Board", fields: { rows: '[{"name":"Ada"},{"name":"Bo"}]' }, fieldDefs: [boardDef], ...patch });
  const withBoard = (bindings?: TemplateScreenLayer["bindings"]): ScreenDoc => ({
    ...doc,
    layers: [
      ...doc.layers,
      { kind: "template", id: "board", name: "Board", templateId: "t1", fields: {}, vars: {}, bindings, rect: { x: 0, y: 0, w: 10, h: 10 } },
    ],
  });
  const sheet = () => (globalThis as { __sheet?: SheetEditorProps }).__sheet!;
  const line = (el: Element) => el.closest(".summary")?.textContent?.replace(/\s+/g, " ").trim();

  async function expand() {
    await fireEvent.click(screen.getByRole("button", { name: "Fields and hotkey of Board" }));
  }

  it("own rows: one summary line; the drawer's edits go out through setLayerFields, debounced 150 ms", async () => {
    api.getScreen.mockImplementation(async (id: string) => ({ doc: { ...withBoard(), id }, live: s1(), warnings: [] }));
    api.setLayerFields.mockImplementation(async (_s: string, _l: string, f: Record<string, string>) => board({ fields: f }));
    await mount([view("s1", [layer("logo"), layer("lower"), board()])]);
    await expand();
    const edit = await within(row("board")).findByRole("button", { name: "Edit items of Rows" }, { timeout: 5000 });
    expect(line(edit)).toBe("2 items · Edit");
    expect(within(row("board")).queryByRole("table")).toBeNull();

    await fireEvent.click(edit);
    await screen.findByRole("dialog", { name: "Rows" }, { timeout: 5000 });
    await screen.findByRole("grid", { name: "Rows" }, { timeout: 5000 });
    expect(sheet().limits.rows).toBe(100);

    vi.useFakeTimers();
    await fireEvent.click(screen.getByRole("button", { name: "Stub add row" }));
    await fireEvent.click(screen.getByRole("button", { name: "Stub delete first" }));
    await vi.advanceTimersByTimeAsync(149);
    expect(api.setLayerFields).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    // One call with the last value (the stub deleted the first of the rows it was opened with).
    expect(api.setLayerFields).toHaveBeenCalledExactlyOnceWith("s1", "board", { rows: '[{"name":"Bo"}]' });
    vi.useRealTimers();

    await fireEvent.keyDown(screen.getByRole("button", { name: "Stub add row" }), { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(document.activeElement).toBe(edit);
  });

  it("linked: 'Linked to: Guests · Edit source' opens the data table; saves go to the source and the list reloads", async () => {
    api.getScreen.mockImplementation(async (id: string) => ({
      doc: { ...withBoard({ rows: { sourceId: "guests", column: "" } }), id },
      live: s1(),
      warnings: [],
    }));
    api.getSource.mockResolvedValue({
      id: "guests",
      name: "Guests",
      kind: "text",
      columns: COLS,
      rows: [{ name: "Ada" }, { name: "Bo" }, { name: "Cy" }],
      index: 1,
      updatedAt: "2026-01-01T00:00:00Z",
    });
    api.saveSource.mockImplementation(async (d: unknown) => d);
    await mount([view("s1", [layer("logo"), layer("lower"), board()])]);
    await expand();
    const edit = await within(row("board")).findByRole("button", { name: "Edit data table Guests" }, { timeout: 5000 });
    expect(line(edit)).toBe("Linked to: Guests · Edit source");

    await fireEvent.click(edit);
    await screen.findByRole("grid", { name: "Guests" }, { timeout: 5000 });
    // s1 has two layers bound to Guests (Lower third's title and this list).
    expect(await screen.findByText("Changes affect 2 layers.")).toBeTruthy();
    expect(sheet().editableColumns ?? false).toBe(false);
    api.listSources.mockClear();
    await fireEvent.click(screen.getByRole("button", { name: "Stub add row" }));
    await fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(api.saveSource).toHaveBeenCalledTimes(1));
    expect(api.saveSource.mock.calls[0][0]).toMatchObject({
      id: "guests",
      rows: [{ name: "Ada" }, { name: "Bo" }, { name: "Cy" }, { name: "New" }],
      index: 1,
    });
    expect(api.setLayerFields).not.toHaveBeenCalled();
    await waitFor(() => expect(api.listSources).toHaveBeenCalled());
  });
});

describe("Control right panel", () => {
  const body = () => document.querySelector<HTMLElement>(".control .body")!;

  it("restores the saved width; a divider sits between stage and panel", async () => {
    await mount([s1()], control({ panelWidth: 500 }));
    expect(body().style.getPropertyValue("--panel-w")).toBe("500px");
    const sep = screen.getByRole("separator", { name: "Resize the side panel" });
    expect(sep.getAttribute("aria-valuenow")).toBe("500");
    expect(sep.getAttribute("aria-controls")).toBe("control-side");
    expect(sep.nextElementSibling?.id).toBe("control-side");
  });

  it("no saved width: the default column (no --panel-w)", async () => {
    await mount([s1()]);
    expect(body().style.getPropertyValue("--panel-w")).toBe("");
  });

  it("keys resize and save the width once (uiState.control.panelWidth)", async () => {
    await mount([s1()], control({ panelWidth: 400 }));
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const sep = screen.getByRole("separator", { name: "Resize the side panel" });
    await fireEvent.keyDown(sep, { key: "ArrowRight" });
    await fireEvent.keyDown(sep, { key: "ArrowRight" });
    await vi.advanceTimersByTimeAsync(400);
    expect(api.setUiState).toHaveBeenCalledTimes(1);
    expect(lastUi().panelWidth).toBe(368);
    await vi.advanceTimersByTimeAsync(0);
    expect(body().style.getPropertyValue("--panel-w")).toBe("368px");
  });

  it("a pointer drag saves once on release, clamped to 640", async () => {
    await mount([s1()], control({ panelWidth: 400 }));
    const sep = screen.getByRole("separator", { name: "Resize the side panel" });
    await fireEvent.pointerDown(sep, { button: 0, clientX: 1000, pointerId: 1 });
    await fireEvent.pointerMove(sep, { clientX: 900, pointerId: 1 });
    await fireEvent.pointerMove(sep, { clientX: 100, pointerId: 1 });
    expect(api.setUiState).not.toHaveBeenCalled();
    await fireEvent.pointerUp(sep, { clientX: 100, pointerId: 1 });
    expect(api.setUiState).toHaveBeenCalledTimes(1);
    expect(lastUi().panelWidth).toBe(640);
  });

  it("a failed save shows the stored width again", async () => {
    api.setUiState.mockRejectedValueOnce(new Error("disk full"));
    await mount([s1()], control({ panelWidth: 400 }));
    const sep = screen.getByRole("separator", { name: "Resize the side panel" });
    await fireEvent.pointerDown(sep, { button: 0, clientX: 1000, pointerId: 1 });
    await fireEvent.pointerMove(sep, { clientX: 900, pointerId: 1 });
    await fireEvent.pointerUp(sep, { clientX: 900, pointerId: 1 });
    await flush();
    await flush();
    expect(body().style.getPropertyValue("--panel-w")).toBe("400px");
    expect(toasts.list.at(-1)?.message).toBe("Couldn't save the panel width: disk full");
  });
});

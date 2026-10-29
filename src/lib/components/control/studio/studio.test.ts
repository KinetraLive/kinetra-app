import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ControlViewState,
  LayerCue,
  LiveLayerView,
  LiveScreenView,
  PreviewView,
  Project,
  ServerStatus,
  StudioOp,
  StudioSettings,
  UiState,
} from "$lib/types";

// ---- Mocks (as control.test.ts, plus the studio commands) ---------------------------------------

// Lazily loaded components these tests open: loaded once here, so the tests wait on them, not on
// Vite compiling them on first use (docs/TESTING.md).
beforeAll(async () => {
  await Promise.all([
    import("$lib/components/control/PanelDivider.svelte"),
    import("$lib/components/recipes/Automation.svelte"),
    import("$lib/components/control/SourcesStrip.svelte"),
  ]);
});

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Handler>() }));
const api = vi.hoisted(() => ({
  remoteView: vi.fn(async () => ({ state: "off", on: false, takeLocked: false, crew: [], showId: null, crewUrl: null, controllerUrl: null, serverVersion: null, rttMs: null, licenseWarning: null, lastError: null, hasShowId: false, available: true, screensHidden: 0 })),
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
  setStudio: vi.fn(),
  studioOp: vi.fn(),
  previewView: vi.fn(),
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
  listRecipeInstances: vi.fn(),
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
vi.mock("$app/navigation", () => ({ goto: vi.fn(), afterNavigate: vi.fn() }));
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/control") } }));

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
vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => setTimeout(() => cb(performance.now()), 0));
vi.stubGlobal("cancelAnimationFrame", (id: number) => clearTimeout(id));

import ControlPage from "../../../../routes/(app)/control/+page.svelte";
import { registry, runCommand, titleOf } from "$lib/commands/registry.svelte";
import { installShortcuts } from "$lib/commands/testing";
import { live } from "$lib/stores/live.svelte";
import { previews } from "$lib/stores/preview.svelte";
import { project } from "$lib/stores/project.svelte";
import { server } from "$lib/stores/server.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { uiState } from "$lib/stores/uiState.svelte";
import { loadSourcesStrip } from "../strips";
import { isStudioTake } from "../hotkeys";
import { keysTaken } from "../control.svelte";
import { TBAR_RATE, TbarOps, Throttle } from "./tbar";

// ---- Fixtures -----------------------------------------------------------------------------------

const running = { state: "running", port: 7770, url: "http://127.0.0.1:7770", lanUrl: null } satisfies ServerStatus;
const current: Project = {
  path: "C:\\shows\\studio",
  name: "Studio Show",
  createdAt: "2026-01-02T10:00:00Z",
  settings: { port: 7770, lanAccess: false, defaultResolution: { width: 1920, height: 1080 } },
};

const STUDIO: StudioSettings = { enabled: true, previewAfterTake: "keep", transition: { kind: "outIn", ms: 500 } };

const control = (patch: Partial<ControlViewState> = {}): UiState => ({
  sidebarPinned: false,
  control: {
    view: "single",
    gridColumns: 2,
    selectedScreenId: "s1",
    hiddenScreenIds: [],
    liveEdit: "onTake",
    hotkeys: {},
    automationOff: false,
    ...patch,
  },
});
const studio = (patch: Partial<ControlViewState> = {}) => control({ studio: STUDIO, ...patch });

function layer(id: string, patch: Partial<LiveLayerView> = {}): LiveLayerView {
  return {
    id,
    name: id === "lower" ? "Lower third" : id === "logo" ? "Logo" : id,
    onAir: false,
    fields: id === "lower" ? { name: "Grace" } : {},
    fieldDefs: id === "lower" ? [{ key: "name", label: "Name", type: "text", default: "" }] : [],
    pending: false,
    ...patch,
  };
}
function view(id: string, layers: LiveLayerView[] = []): LiveScreenView {
  return { screen: { id, name: `Screen ${id}`, width: 1920, height: 1080, background: "transparent" }, layers };
}
/** Program: the lower third on air, the logo off. */
const s1 = () => view("s1", [layer("logo"), layer("lower", { onAir: true })]);

/** A Preview of s1 with these cues (layers follow the cues). */
function pv(cues: Partial<LayerCue>[] = [], patch: Partial<PreviewView> = {}, screenId = "s1"): PreviewView {
  const full = cues.map((c) => ({ layerId: "", visible: null, staged: false, programChanged: false, ...c }));
  const base = screenId === "s1" ? s1() : view(screenId);
  return {
    screenId,
    rev: 1,
    cues: full,
    layers: base.layers.map((l) => {
      const c = full.find((x) => x.layerId === l.id);
      return c && c.visible !== null ? { ...l, onAir: c.visible } : l;
    }),
    tbar: null,
    ...patch,
  };
}

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

async function settle() {
  for (let i = 0; i < 4; i++) await flush();
  await intersectAll();
}

async function mount(screens: LiveScreenView[], ui: UiState) {
  api.listScreens.mockResolvedValue(screens);
  uiState.set(ui);
  const r = render(ControlPage);
  await flush();
  await loadSourcesStrip();
  await import("../LayerBody.svelte");
  await import("../commands.svelte");
  await import("./ui.svelte");
  await import("./StudioExit.svelte");
  await settle();
  return r;
}

const frames = () => [...document.querySelectorAll("iframe")];
const ops = (): StudioOp[] => api.studioOp.mock.calls.map((c) => c[1] as StudioOp);
const studioSwitch = () => screen.getByRole("switch", { name: "Studio mode" });
const commands = () => registry.list.map(titleOf);
const cmd = (title: string) => registry.list.find((c) => titleOf(c) === title)!.id;

/** The server: replies to studio ops with this screen's Preview. */
let serverPv: PreviewView;

beforeEach(() => {
  vi.clearAllMocks();
  obs.io.length = 0;
  events.handlers.clear();
  server.status = null;
  live.screens = new Map();
  previews.clear();
  api.on.mockImplementation((name: string, h: Handler) => {
    events.handlers.set(name, h);
    return () => events.handlers.delete(name);
  });
  api.serverStatus.mockResolvedValue(running);
  api.getScreen.mockImplementation(async (id: string) => ({ doc: { id, layers: [] }, live: s1(), warnings: [] }));
  api.listSources.mockResolvedValue([]);
  api.listTemplates.mockResolvedValue([]);
  api.setUiState.mockImplementation(async (v: UiState) => v);
  api.setStudio.mockResolvedValue(undefined);
  serverPv = pv();
  api.previewView.mockImplementation(async (id: string) => (id === "s1" ? serverPv : pv([], {}, id)));
  api.studioOp.mockImplementation(async (_id: string, op: StudioOp) => ({ ...serverPv, tbar: op.op === "tbarSet" ? op.p : serverPv.tbar }));
  api.liveShow.mockImplementation(async (_s: string, l: string) => layer(l, { onAir: true }));
  api.livePanic.mockResolvedValue([s1()]);
  api.listFlows.mockResolvedValue([]);
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

// ---- The switch and the exit question ------------------------------------------------------------

describe("Studio mode switch", () => {
  it("turns on through set_studio (local settings follow) and shows Preview and Program", async () => {
    await mount([s1()], control());
    expect(frames()).toHaveLength(1);
    expect(studioSwitch().getAttribute("aria-checked")).toBe("false");
    await fireEvent.click(studioSwitch());
    expect(api.setStudio).toHaveBeenCalledWith(
      { enabled: true, previewAfterTake: "keep", transition: { kind: "crossfade", ms: 500 } },
      null,
    );
    expect(api.setUiState).not.toHaveBeenCalled();
    expect(uiState.value?.control.studio?.enabled).toBe(true);
    await settle();
    expect(studioSwitch().getAttribute("aria-checked")).toBe("true");
    expect(frames().map((f) => [f.title, f.src])).toEqual([
      ["Screen s1 Preview", "http://127.0.0.1:7770/screen/s1/preview?mute=1"],
      ["Screen s1 Program", "http://127.0.0.1:7770/screen/s1?mute=1"],
    ]);
    expect(api.previewView).toHaveBeenCalledWith("s1");
  });

  it("a refused switch rolls the local settings back and says why", async () => {
    api.setStudio.mockRejectedValue({ code: "internal", message: "disk full" });
    await mount([s1()], control());
    await fireEvent.click(studioSwitch());
    await settle();
    expect(uiState.value?.control.studio).toBeUndefined();
    expect(toasts.list.at(-1)?.message).toBe("Couldn't turn Studio mode on: disk full");
  });

  it("off with an empty Preview: straight off, no question", async () => {
    await mount([s1()], studio());
    await fireEvent.click(studioSwitch());
    await settle();
    expect(api.setStudio).toHaveBeenCalledWith({ ...STUDIO, enabled: false }, null);
    expect(screen.queryByRole("dialog", { name: "Turn Studio mode off?" })).toBeNull();
    expect(frames()).toHaveLength(1);
  });

  it.each([
    ["Take", "take"],
    ["Keep for next take", "keepStaged"],
    ["Discard", "discard"],
  ] as const)("off with a pending Preview asks first: %s", async (button, exit) => {
    serverPv = pv([{ layerId: "logo", visible: true }]);
    await mount([s1()], studio());
    await fireEvent.click(studioSwitch());
    await settle();
    const dlg = screen.getByRole("dialog", { name: "Turn Studio mode off?" });
    expect(dlg.textContent).toContain("The Preview of Screen s1 differs from Program.");
    expect(api.setStudio).not.toHaveBeenCalled();
    await fireEvent.click(within(dlg).getByRole("button", { name: button }));
    await settle();
    expect(api.setStudio).toHaveBeenCalledWith({ ...STUDIO, enabled: false }, exit);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(uiState.value?.control.studio?.enabled).toBe(false);
  });

  it("Cancel keeps Studio mode on", async () => {
    serverPv = pv([{ layerId: "logo", visible: true }]);
    await mount([s1()], studio());
    await fireEvent.click(studioSwitch());
    await settle();
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await settle();
    expect(api.setStudio).not.toHaveBeenCalled();
    expect(studioSwitch().getAttribute("aria-checked")).toBe("true");
  });

  it("a refusal because of a Preview it didn't know yet asks too", async () => {
    await mount([s1()], studio());
    serverPv = pv([{ layerId: "logo", visible: true }]);
    api.setStudio.mockRejectedValueOnce({ code: "invalid", message: "Preview has changes" });
    await fireEvent.click(studioSwitch());
    await settle();
    expect(screen.getByRole("dialog", { name: "Turn Studio mode off?" })).toBeTruthy();
    expect(studioSwitch().getAttribute("aria-checked")).toBe("true");
  });

  it("the settings popover saves Preview after take and the default transition", async () => {
    await mount([s1()], studio());
    await fireEvent.click(screen.getByRole("button", { name: "Studio mode settings" }));
    await settle();
    const pop = within(await screen.findByRole("dialog", { name: "Studio mode" }));
    await fireEvent.click(pop.getByRole("radio", { name: "Clear" }));
    expect(api.setStudio).toHaveBeenLastCalledWith({ ...STUDIO, previewAfterTake: "clear" }, null);
    await fireEvent.click(pop.getByRole("radio", { name: "Crossfade" }));
    expect(api.setStudio).toHaveBeenLastCalledWith(
      { ...STUDIO, previewAfterTake: "clear", transition: { kind: "crossfade", ms: 500 } },
      null,
    );
    const ms = pop.getByLabelText("Crossfade duration") as HTMLInputElement;
    await fireEvent.change(ms, { target: { value: "9000" } });
    expect(api.setStudio).toHaveBeenLastCalledWith(
      { ...STUDIO, previewAfterTake: "clear", transition: { kind: "crossfade", ms: 5000 } },
      null,
    );
  });
});

// ---- The single view ------------------------------------------------------------------------------

describe("Studio single view", () => {
  it("exactly two iframes in Studio mode, labelled with tallies and text; one without it", async () => {
    await mount([s1()], studio());
    expect(frames()).toHaveLength(2);
    const pvw = screen.getByRole("region", { name: "Preview" });
    const pgm = screen.getByRole("region", { name: "Program" });
    expect(pvw.textContent).toContain("same as Program");
    expect(pgm.textContent).toContain("1 on air");
    cleanup();
    obs.io.length = 0;
    await mount([s1()], control());
    expect(frames().map((f) => f.src)).toEqual(["http://127.0.0.1:7770/screen/s1?mute=1"]);
  });

  it("the column's transition is the saved default; TAKE uses it (no transition sent)", async () => {
    await mount([s1()], studio());
    const take = screen.getByRole("button", { name: "TAKE" });
    expect(take.title).toBe("Preview to Program (Play out & in) · Space");
    await fireEvent.click(take);
    await settle();
    expect(api.studioOp).toHaveBeenLastCalledWith("s1", { op: "take", transition: null });
    const kinds = within(screen.getByRole("radiogroup", { name: "Transition" }));
    expect((kinds.getByRole("radio", { name: "Play out & in" }) as HTMLElement).getAttribute("aria-checked")).toBe("true");
    await fireEvent.click(kinds.getByRole("radio", { name: "Crossfade" }));
    expect(api.setStudio).toHaveBeenLastCalledWith({ ...STUDIO, transition: { kind: "crossfade", ms: 500 } }, null);
    const ms = screen.getByLabelText("Crossfade duration") as HTMLInputElement;
    await fireEvent.change(ms, { target: { value: "800" } });
    expect(api.setStudio).toHaveBeenLastCalledWith({ ...STUDIO, transition: { kind: "crossfade", ms: 800 } }, null);
    await tick();
    expect(screen.getByRole("button", { name: "TAKE" }).title).toBe("Preview to Program (Crossfade 800 ms) · Space");
    await fireEvent.click(kinds.getByRole("radio", { name: "Cut" }));
    await tick();
    expect(document.body.textContent).toContain("Cut · instant");
    await fireEvent.click(screen.getByRole("button", { name: "TAKE" }));
    await settle();
    expect(api.studioOp).toHaveBeenLastCalledWith("s1", { op: "take", transition: null });
  });

  it("shows the server's T-bar position and the number of changes", async () => {
    serverPv = pv([{ layerId: "logo", visible: true }], { tbar: 0.45 });
    await mount([s1()], studio());
    expect(screen.getByRole("slider", { name: "T-bar" }).getAttribute("aria-valuenow")).toBe("45");
    expect(screen.getByRole("region", { name: "Preview" }).textContent).toContain("1 change");
    events.handlers.get("preview-changed")!(pv([], { rev: 2, tbar: null }));
    await tick();
    expect(screen.getByRole("slider", { name: "T-bar" }).getAttribute("aria-valuenow")).toBe("0");
  });

  async function slider() {
    const el = screen.getByRole("slider", { name: "T-bar" });
    el.getBoundingClientRect = () => ({ top: 0, bottom: 200, height: 200, left: 0, right: 40, width: 40, x: 0, y: 0, toJSON() {} });
    return el;
  }

  it("T-bar by pointer: grab begins, moves set the position, release at the top commits", async () => {
    await mount([s1()], studio());
    const el = await slider();
    await fireEvent.pointerDown(el, { button: 0, clientY: 200, pointerId: 1 });
    await fireEvent.pointerMove(el, { clientY: 100, pointerId: 1 });
    await new Promise((r) => setTimeout(r, 60));
    expect(el.getAttribute("aria-valuenow")).toBe("50");
    await fireEvent.pointerMove(el, { clientY: 0, pointerId: 1 });
    await fireEvent.pointerUp(el, { clientY: 0, pointerId: 1 });
    await settle();
    const seen = ops();
    expect(seen[0]).toEqual({ op: "tbarBegin", transition: null });
    expect(seen.at(-1)).toEqual({ op: "tbarEnd", commit: true });
    const sets = seen.filter((o) => o.op === "tbarSet").map((o) => (o as { p: number }).p);
    expect(sets[0]).toBe(0);
    expect(sets).toContain(0.5);
  });

  it("T-bar released short of the top aborts (Program unchanged)", async () => {
    await mount([s1()], studio());
    const el = await slider();
    await fireEvent.pointerDown(el, { button: 0, clientY: 150, pointerId: 1 });
    await fireEvent.pointerUp(el, { clientY: 200, pointerId: 1 });
    await settle();
    expect(ops().at(-1)).toEqual({ op: "tbarEnd", commit: false });
    expect(el.getAttribute("aria-valuenow")).toBe("0");
  });

  it("T-bar by keyboard: arrows ±5 %, PageUp/PageDown ±25 %, End commits, Home aborts", async () => {
    await mount([s1()], studio());
    const el = await slider();
    el.focus();
    await fireEvent.keyDown(el, { key: "ArrowUp" });
    await fireEvent.keyDown(el, { key: "PageUp" });
    await fireEvent.keyDown(el, { key: "ArrowDown" });
    expect(el.getAttribute("aria-valuenow")).toBe("25");
    await fireEvent.keyDown(el, { key: "End" });
    await settle();
    let seen = ops();
    expect(seen[0]).toEqual({ op: "tbarBegin", transition: null });
    expect(seen.at(-1)).toEqual({ op: "tbarEnd", commit: true });
    expect(seen.filter((o) => o.op === "tbarBegin")).toHaveLength(1);

    api.studioOp.mockClear();
    await fireEvent.keyDown(el, { key: "PageUp" });
    await fireEvent.keyDown(el, { key: "Home" });
    await settle();
    seen = ops();
    expect(seen[0].op).toBe("tbarBegin");
    expect(seen.at(-1)).toEqual({ op: "tbarEnd", commit: false });

    // Esc lets go mid-way (abort); a key at 0 going down doesn't begin.
    api.studioOp.mockClear();
    await fireEvent.keyDown(el, { key: "ArrowDown" });
    expect(ops()).toEqual([]);
    await fireEvent.keyDown(el, { key: "ArrowUp" });
    await fireEvent.keyDown(el, { key: "Escape" });
    await settle();
    expect(ops().at(-1)).toEqual({ op: "tbarEnd", commit: false });
  });
});

// ---- Layer rows ----------------------------------------------------------------------------------

describe("Studio layer rows", () => {
  const row = (id: string) => document.querySelector<HTMLElement>(`[data-layer="${id}"]`)!;

  it("Program and Preview tallies with their state; Cue in / Cue out instead of Take / Clear", async () => {
    serverPv = pv([{ layerId: "lower", visible: false }]);
    await mount([s1()], studio());
    const lower = within(row("lower"));
    expect(lower.getByRole("img", { name: "On air" })).toBeTruthy();
    expect(lower.getByRole("img", { name: "Not in Preview" })).toBeTruthy();
    expect(lower.getByRole("button", { name: "Cue in Lower third" })).toBeTruthy();
    expect(lower.queryByRole("button", { name: "Take Lower third" })).toBeNull();
    const logo = within(row("logo"));
    expect(logo.getByRole("img", { name: "Off air" })).toBeTruthy();
    expect(logo.getByRole("button", { name: "Cue in Logo" })).toBeTruthy();
  });

  it("a cue toggles at once and goes through studio_op; back to Program state = as Program", async () => {
    await mount([s1()], studio());
    let resolve!: (v: PreviewView) => void;
    api.studioOp.mockImplementationOnce(() => new Promise((r) => (resolve = r)));
    await fireEvent.click(within(row("logo")).getByRole("button", { name: "Cue in Logo" }));
    // Optimistic: the row says it is in Preview before the reply.
    expect(within(row("logo")).getByRole("button", { name: "Cue out Logo" })).toBeTruthy();
    expect(within(row("logo")).getByRole("img", { name: "In Preview" })).toBeTruthy();
    expect(api.studioOp).toHaveBeenLastCalledWith("s1", { op: "cue", layerId: "logo", visible: true });
    resolve(pv([{ layerId: "logo", visible: true }], { rev: 2 }));
    await settle();
    expect(within(row("logo")).getByRole("button", { name: "Cue out Logo" })).toBeTruthy();
    // Cued out again: that is what Program shows, so the cue is cleared (null).
    await fireEvent.click(within(row("logo")).getByRole("button", { name: "Cue out Logo" }));
    expect(api.studioOp).toHaveBeenLastCalledWith("s1", { op: "cue", layerId: "logo", visible: null });
    // An on-air layer cued out of Preview.
    await fireEvent.click(within(row("lower")).getByRole("button", { name: "Cue out Lower third" }));
    expect(api.studioOp).toHaveBeenLastCalledWith("s1", { op: "cue", layerId: "lower", visible: false });
  });

  it("a failed cue rolls back and says why", async () => {
    await mount([s1()], studio());
    api.studioOp.mockRejectedValueOnce({ code: "notFound", message: "Layer not found" });
    await fireEvent.click(within(row("logo")).getByRole("button", { name: "Cue in Logo" }));
    await settle();
    expect(within(row("logo")).getByRole("button", { name: "Cue in Logo" })).toBeTruthy();
    expect(toasts.list.at(-1)?.message).toBe("Couldn't cue Logo: Layer not found");
  });

  it("markers: In preview (staged edits) and Program changed since cue", async () => {
    serverPv = pv([
      { layerId: "lower", visible: null, staged: true },
      { layerId: "logo", visible: true, programChanged: true },
    ]);
    await mount([s1()], studio());
    expect(row("lower").textContent).toContain("In preview");
    expect(row("lower").textContent).not.toContain("Program changed since cue");
    expect(row("logo").textContent).toContain("Program changed since cue");
    expect(row("logo").querySelector(".changed")).toBeTruthy();
  });

  it("the fields show the layer as it is in Preview", async () => {
    serverPv = { ...pv([{ layerId: "lower", visible: null, staged: true }]) };
    serverPv.layers = serverPv.layers.map((l) => (l.id === "lower" ? { ...l, fields: { name: "Ada" } } : l));
    await mount([s1()], studio());
    await fireEvent.click(within(row("lower")).getByRole("button", { name: "Fields and hotkey of Lower third" }));
    await settle();
    expect((within(row("lower")).getByLabelText("Name") as HTMLInputElement).value).toBe("Ada");
  });

  it("Take preview in the panel header (for the grid view), enabled while the Preview differs", async () => {
    await mount([s1()], studio({ view: "grid" }));
    const btn = screen.getByRole("button", { name: "Take the Preview of Screen s1" }) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    events.handlers.get("preview-changed")!(pv([{ layerId: "logo", visible: true }], { rev: 2 }));
    await tick();
    expect(btn.disabled).toBe(false);
    await fireEvent.click(btn);
    await settle();
    expect(api.studioOp).toHaveBeenLastCalledWith("s1", { op: "take", transition: null });
  });
});

// ---- Grid ------------------------------------------------------------------------------------------

describe("Studio grid", () => {
  it("Program only; a Preview pending badge (text) on screens whose Preview differs", async () => {
    api.previewView.mockImplementation(async (id: string) =>
      id === "s2" ? pv([{ layerId: "x", visible: true }], {}, "s2") : pv([], {}, id),
    );
    await mount([s1(), view("s2"), view("s3")], studio({ view: "grid" }));
    expect(frames().every((f) => !f.src.endsWith("/preview"))).toBe(true);
    const tile = (id: string) => document.querySelector<HTMLElement>(`[data-screen="${id}"]`)!;
    expect(tile("s2").textContent).toContain("Preview pending");
    expect(tile("s1").textContent).not.toContain("Preview pending");
    expect(tile("s3").textContent).not.toContain("Preview pending");
  });

  it("no badge outside Studio mode", async () => {
    await mount([s1(), view("s2")], control({ view: "grid" }));
    expect(document.body.textContent).not.toContain("Preview pending");
    expect(api.previewView).not.toHaveBeenCalled();
  });
});

// ---- Hotkeys and the palette -------------------------------------------------------------------------

describe("Studio hotkeys", () => {
  it("Space and Enter take the selected screen's Preview when nothing else uses the key", async () => {
    await mount([s1()], studio());
    expect(await fireEvent.keyDown(document.body, { key: " " })).toBe(false);
    await settle();
    expect(api.studioOp).toHaveBeenLastCalledWith("s1", { op: "take", transition: null });
    await fireEvent.keyDown(document.body, { key: "Enter" });
    await settle();
    expect(api.studioOp).toHaveBeenCalledTimes(2);
  });

  it("never while typing, on a button or slider, in a dialog, on repeat or with modifiers", async () => {
    await mount([s1()], studio());
    await fireEvent.click(within(document.querySelector<HTMLElement>('[data-layer="lower"]')!).getByRole("button", { name: "Fields and hotkey of Lower third" }));
    await settle();
    api.studioOp.mockClear();
    await fireEvent.keyDown(screen.getByLabelText("Name"), { key: " " });
    await fireEvent.keyDown(screen.getByRole("radio", { name: "Cut" }), { key: "Enter" });
    await fireEvent.keyDown(screen.getByRole("slider", { name: "T-bar" }), { key: "Enter" });
    await fireEvent.keyDown(document.body, { key: " ", repeat: true });
    await fireEvent.keyDown(document.body, { key: "Enter", ctrlKey: true });
    const dlg = document.createElement("div");
    dlg.setAttribute("role", "dialog");
    const inner = document.createElement("span");
    dlg.append(inner);
    document.body.append(dlg);
    await fireEvent.keyDown(inner, { key: " " });
    dlg.remove();
    await settle();
    expect(ops().filter((o) => o.op === "take")).toEqual([]);
  });

  it("not outside Studio mode", async () => {
    await mount([s1()], control());
    await fireEvent.keyDown(document.body, { key: " " });
    await settle();
    expect(api.studioOp).not.toHaveBeenCalled();
  });

  it("a layer's F-key cues it in Preview instead of taking it; Shift+Esc is still PANIC", async () => {
    await mount([s1()], studio({ hotkeys: { "s1/logo": "F1" } }));
    await fireEvent.keyDown(window, { key: "F1" });
    await settle();
    expect(api.studioOp).toHaveBeenLastCalledWith("s1", { op: "cue", layerId: "logo", visible: true });
    expect(api.liveShow).not.toHaveBeenCalled();
    await fireEvent.keyDown(window, { key: "Escape", shiftKey: true });
    await settle();
    expect(api.livePanic).toHaveBeenCalled();
  });

  it("isStudioTake and keysTaken", () => {
    const k = (key: string, more: Partial<KeyboardEvent> = {}) =>
      ({ key, shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, repeat: false, ...more }) as KeyboardEvent;
    expect(isStudioTake(k(" "), false)).toBe(true);
    expect(isStudioTake(k("Enter"), false)).toBe(true);
    expect(isStudioTake(k("Enter"), true)).toBe(false);
    expect(isStudioTake(k(" ", { shiftKey: true }), false)).toBe(false);
    expect(isStudioTake(k("a"), false)).toBe(false);
    const b = document.createElement("button");
    const d = document.createElement("dialog");
    const s = document.createElement("span");
    d.append(s);
    expect(keysTaken(b)).toBe(true);
    expect(keysTaken(s)).toBe(true);
    expect(keysTaken(document.body)).toBe(false);
    expect(keysTaken(null)).toBe(false);
  });
});

describe("Studio palette", () => {
  it("Studio mode on/off, Take preview (selected and per screen), Transition: …", async () => {
    await mount([s1(), view("s2")], control());
    expect(commands()).toContain("Studio mode on");
    expect(commands()).not.toContain("Take preview");
    runCommand(cmd("Studio mode on"));
    await settle();
    expect(api.setStudio).toHaveBeenLastCalledWith(expect.objectContaining({ enabled: true }), null);
    await waitFor(() => expect(commands()).toContain("Studio mode off"));
    expect(commands()).toEqual(
      expect.arrayContaining([
        "Take preview",
        "Take preview (Screen s1)",
        "Take preview (Screen s2)",
        "Transition: Cut",
        "Transition: Crossfade",
        "Transition: Play out & in",
      ]),
    );
    runCommand(cmd("Take preview (Screen s2)"));
    await settle();
    expect(api.studioOp).toHaveBeenLastCalledWith("s2", { op: "take", transition: null });
    runCommand(cmd("Take preview"));
    await settle();
    expect(api.studioOp).toHaveBeenLastCalledWith("s1", { op: "take", transition: null });
    // "Transition: …" saves the project default.
    runCommand(cmd("Transition: Cut"));
    await settle();
    expect(api.setStudio).toHaveBeenLastCalledWith(
      { enabled: true, previewAfterTake: "keep", transition: { kind: "cut", ms: 500 } },
      null,
    );
    expect(uiState.value?.control.studio?.transition.kind).toBe("cut");
  });

  it("leaving Control removes the commands", async () => {
    const r = await mount([s1()], studio());
    expect(commands()).toContain("Take preview");
    r.unmount();
    expect(commands().filter((t) => t.startsWith("Take preview") || t.startsWith("Studio mode"))).toEqual([]);
  });
});

// ---- Wire side: the T-bar rate and the Preview store ----------------------------------------------------

describe("T-bar wire", () => {
  it(`positions go out at most ${TBAR_RATE}/s, the last one always`, () => {
    vi.useFakeTimers();
    const sent: number[] = [];
    const at: number[] = [];
    const t = new Throttle<number>(TBAR_RATE, (p) => {
      sent.push(p);
      at.push(performance.now());
    });
    // 120 moves over one second (a 120 Hz pointer).
    for (let i = 1; i <= 120; i++) {
      t.push(i / 120);
      vi.advanceTimersByTime(1000 / 120);
    }
    vi.advanceTimersByTime(100);
    // Never two within 1/30 s; about 30 in the second.
    for (let i = 1; i < at.length; i++) expect(at[i] - at[i - 1]).toBeGreaterThanOrEqual(1000 / TBAR_RATE - 0.01);
    expect(at.filter((x) => x - at[0] < 1000).length).toBeLessThanOrEqual(TBAR_RATE);
    expect(sent.length).toBeGreaterThan(TBAR_RATE - 3);
    expect(sent.at(-1)).toBe(1);
  });

  it("ops are sent in order: begin, positions (one waiting is dropped), then the end", async () => {
    vi.useFakeTimers();
    const log: StudioOp[] = [];
    const tb = new TbarOps(async (op) => void log.push(op));
    tb.set(0.2); // not begun: ignored
    tb.begin({ kind: "crossfade", ms: 500 });
    tb.set(0.1);
    tb.set(0.3);
    tb.set(0.9);
    await tb.end(true);
    expect(log).toEqual([
      { op: "tbarBegin", transition: { kind: "crossfade", ms: 500 } },
      { op: "tbarSet", p: 0.1 },
      { op: "tbarEnd", commit: true },
    ]);
    tb.begin(null);
    tb.dispose();
    await vi.runAllTimersAsync();
    expect(log.at(-1)).toEqual({ op: "tbarEnd", commit: false });
  });
});

describe("Preview store", () => {
  it("an older view never replaces a newer one; optimistic views can", () => {
    previews.put(pv([], { rev: 5 }));
    previews.put(pv([{ layerId: "logo", visible: true }], { rev: 4 }));
    expect(previews.pending("s1")).toBe(false);
    previews.put(pv([{ layerId: "logo", visible: true }], { rev: 4 }), true);
    expect(previews.pending("s1")).toBe(true);
    previews.clear();
    expect(previews.views.size).toBe(0);
  });
});

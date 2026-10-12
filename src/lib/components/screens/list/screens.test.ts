import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../../templates/list/matchers";
import type { LiveScreenView, Project, ScreenState, ScreenSummary, ServerStatus } from "$lib/types";

// ---- Mocks --------------------------------------------------------------------------------------

type Handler = (payload: unknown) => void;

const events = vi.hoisted(() => ({ handlers: new Map<string, Handler>() }));
const api = vi.hoisted(() => ({
  serverStatus: vi.fn(),
  listScreens: vi.fn(),
  listScreenDocs: vi.fn(),
  createScreen: vi.fn(),
  renameScreen: vi.fn(),
  duplicateScreen: vi.fn(),
  deleteScreen: vi.fn(),
  getScreen: vi.fn(),
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  on: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e
      ? e
      : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
// Domain modules share the same mock functions (the split only changes where they are imported from).
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/screens", () => api);

const nav = vi.hoisted(() => ({ goto: vi.fn() }));
vi.mock("$app/navigation", () => ({ goto: nav.goto, afterNavigate: vi.fn() }));
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/screens") } }));

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

/** Every observed thumbnail box enters (or leaves) the viewport. */
async function intersect(isIntersecting: boolean) {
  for (const rec of obs.io) {
    if (rec.off || rec.targets.length === 0) continue;
    const entries = rec.targets.map((target) => ({ target, isIntersecting }));
    rec.cb(entries as unknown as IntersectionObserverEntry[], {} as IntersectionObserver);
  }
  await tick();
}

import Page from "../../../../routes/(app)/screens/+page.svelte";
import NewScreenDialog from "../new/NewScreenDialog.svelte";
import {
  defaultScreenName,
  presetFor,
  validHex,
  validScreenName,
  validSide,
} from "../new/format";
import { fitScale, layersLabel, sortScreens, tallyOf } from "./format";
import { project } from "$lib/stores/project.svelte";
import { server } from "$lib/stores/server.svelte";
import { toasts } from "$lib/stores/toasts.svelte";

// ---- Fixtures -----------------------------------------------------------------------------------

const running: ServerStatus = {
  state: "running",
  port: 7770,
  url: "http://127.0.0.1:7770",
  lanUrl: null,
};

const current: Project = {
  path: "C:\\shows\\current",
  name: "Current Show",
  createdAt: "2026-01-02T10:00:00Z",
  settings: { port: 7770, lanAccess: false, defaultResolution: { width: 1280, height: 720 } },
};

function summary(id: string, name: string, extra: Partial<ScreenSummary> = {}): ScreenSummary {
  return {
    id,
    name,
    width: 1920,
    height: 1080,
    background: "transparent",
    layerCount: 2,
    updatedAt: "2026-10-01T10:00:00Z",
    ...extra,
  };
}

function view(id: string, onAir = false): LiveScreenView {
  return {
    screen: { id, name: id, width: 1920, height: 1080, background: "transparent" },
    layers: [{ id: "l", name: "Layer", onAir, fields: {}, fieldDefs: [], pending: false }],
  };
}

function state(id: string, name: string): ScreenState {
  return {
    doc: {
      id,
      name,
      width: 1920,
      height: 1080,
      background: "transparent",
      layers: [],
      updatedAt: "2026-10-01T10:00:00Z",
    },
    live: view(id),
    warnings: [],
  };
}

const main = summary("m1", "Main program");
const vert = summary("v2", "Vertical", { width: 1080, height: 1920, layerCount: 1 });

beforeEach(() => {
  obs.io.length = 0;
  events.handlers.clear();
  server.status = null;
  project.current = current;
  api.on.mockImplementation((name: string, h: Handler) => {
    events.handlers.set(name, h);
    return () => events.handlers.delete(name);
  });
  api.serverStatus.mockResolvedValue(running);
  api.listScreenDocs.mockResolvedValue([vert, main]);
  api.listScreens.mockResolvedValue([view("m1", true), view("v2")]);
  api.renameScreen.mockImplementation(async (id: string, name: string) => summary(id, name));
  api.duplicateScreen.mockResolvedValue(state("c3", "Main program copy"));
  api.deleteScreen.mockResolvedValue(undefined);
  api.createScreen.mockResolvedValue(state("n1", "Screen 3"));
});

afterEach(() => {
  cleanup();
  toasts.clear();
  vi.clearAllMocks();
});

const card = (name: string) => {
  const el = [...document.querySelectorAll<HTMLElement>("article[data-screen]")].find(
    (a) => a.querySelector("[data-pick]")?.textContent === name,
  );
  if (!el) throw new Error(`no card ${name}`);
  return el;
};
const frames = () => [...document.querySelectorAll("iframe")];

// ---- Helpers ------------------------------------------------------------------------------------

describe("screens rules", () => {
  it("tally, sort, labels and fit", () => {
    expect(tallyOf(view("a", true), null)).toBe("program");
    expect(tallyOf(view("a"), running)).toBe("preview");
    expect(tallyOf(undefined, running)).toBe("preview");
    expect(tallyOf(undefined, { state: "stopped" })).toBe("idle");
    expect(sortScreens([vert, main]).map((s) => s.id)).toEqual(["m1", "v2"]);
    expect(layersLabel(1)).toBe("1 layer");
    expect(layersLabel(0)).toBe("0 layers");
    expect(fitScale(320, 180, 1920, 1080)).toBeCloseTo(1 / 6);
    expect(fitScale(320, 180, 1080, 1920)).toBeCloseTo(180 / 1920);
    expect(fitScale(0, 180, 1920, 1080)).toBe(0);
  });

  it("new screen validation and defaults", () => {
    expect(validSide(16)).toBe(true);
    expect(validSide(8192)).toBe(true);
    expect(validSide(15)).toBe(false);
    expect(validSide(8193)).toBe(false);
    expect(validSide(100.5)).toBe(false);
    expect(validHex("#ff00aa")).toBe(true);
    expect(validHex("#FF00AA80")).toBe(true);
    expect(validHex("#fff")).toBe(false);
    expect(validHex("red")).toBe(false);
    expect(validScreenName("  ")).toBe(false);
    expect(validScreenName("x".repeat(101))).toBe(false);
    expect(defaultScreenName([])).toBe("Screen 1");
    expect(defaultScreenName(["Main"])).toBe("Screen 2");
    expect(defaultScreenName(["Screen 2", "x"])).toBe("Screen 3");
    expect(presetFor({ width: 1080, height: 1920 })).toBe("vertical");
    expect(presetFor({ width: 1000, height: 1000 })).toBe("custom");
  });
});

// ---- Page ---------------------------------------------------------------------------------------

describe("/screens", () => {
  it("renders a card per screen, sorted, with tally, URL, resolution and layer count", async () => {
    render(Page);
    expect(await screen.findByRole("heading", { level: 1, name: "Screens" })).toBeTruthy();
    await waitFor(() => expect(document.querySelectorAll("article[data-screen]")).toHaveLength(2));
    const [first, second] = document.querySelectorAll<HTMLElement>("article[data-screen]");
    expect(first).toHaveTextContent("Main program");
    expect(within(first).getByLabelText("On air")).toBeTruthy();
    expect(first).toHaveTextContent("1920×1080 · 2 layers");
    await waitFor(() => expect(first).toHaveTextContent("http://127.0.0.1:7770/screen/m1"));
    expect(second).toHaveTextContent("1080×1920 · 1 layer");
    expect(within(second).getByLabelText("Ready")).toBeTruthy();
    // The first is selected (brackets), and only its controls are in the tab order.
    expect(first.hasAttribute("data-selected")).toBe(true);
    expect(second.hasAttribute("data-selected")).toBe(false);
    expect(first.classList.contains("vf-live")).toBe(true);
    expect(within(second).getByRole("button", { name: "Vertical" })).toHaveAttribute("tabindex", "-1");
    expect(document.body.textContent).not.toMatch(/coming|placeholder|sample/i);
  });

  it("shows the LAN URL when LAN access is on", async () => {
    api.serverStatus.mockResolvedValue({ ...running, lanUrl: "http://192.168.1.20:7770" });
    render(Page);
    await waitFor(() =>
      expect(card("Main program")).toHaveTextContent("http://192.168.1.20:7770/screen/m1"),
    );
    // The thumbnail still loads the local URL.
    await intersect(true);
    expect(frames()[0].getAttribute("src")).toBe("http://127.0.0.1:7770/screen/m1?mute=1");
  });

  it("click selects; Enter and double-click open the layout editor", async () => {
    render(Page);
    await waitFor(() => card("Vertical"));
    const pick = within(card("Vertical")).getByRole("button", { name: "Vertical" });
    await fireEvent.click(pick);
    expect(card("Vertical").hasAttribute("data-selected")).toBe(true);
    expect(card("Main program").hasAttribute("data-selected")).toBe(false);
    await fireEvent.keyDown(pick, { key: "Enter" });
    expect(nav.goto).toHaveBeenLastCalledWith("/screens/v2");
    await fireEvent.dblClick(within(card("Main program")).getByRole("button", { name: "Main program" }));
    expect(nav.goto).toHaveBeenLastCalledWith("/screens/m1");
    await fireEvent.click(within(card("Main program")).getByRole("button", { name: "Open Main program" }));
    expect(nav.goto).toHaveBeenCalledTimes(3);
  });

  it("arrow keys move the selection", async () => {
    render(Page);
    await waitFor(() => card("Main program"));
    await fireEvent.click(within(card("Main program")).getByRole("button", { name: "Main program" }));
    await fireEvent.keyDown(card("Main program"), { key: "ArrowRight" });
    expect(card("Vertical").hasAttribute("data-selected")).toBe(true);
    await fireEvent.keyDown(card("Vertical"), { key: "Home" });
    expect(card("Main program").hasAttribute("data-selected")).toBe(true);
  });

  it("live thumbnails: at most 6, only while visible and the server runs", async () => {
    const many = Array.from({ length: 8 }, (_, i) => summary(`s${i}`, `Screen ${i}`));
    api.listScreenDocs.mockResolvedValue(many);
    api.listScreens.mockResolvedValue([]);
    render(Page);
    await waitFor(() => expect(document.querySelectorAll("article[data-screen]")).toHaveLength(8));
    await waitFor(() => expect(server.status?.state).toBe("running"));
    expect(frames()).toHaveLength(0);
    await intersect(true);
    expect(frames()).toHaveLength(6);
    for (const f of frames()) expect(f.getAttribute("src")).toMatch(/^http:\/\/127\.0\.0\.1:7770\/screen\/s[0-5]\?mute=1$/);
    expect(screen.getAllByText("Preview paused")).toHaveLength(2);
    await intersect(false);
    expect(frames()).toHaveLength(0);
    await intersect(true);
    server.status = { state: "stopped" };
    await tick();
    expect(frames()).toHaveLength(0);
  });

  it("empty project: the bracket-framed create action opens the New screen dialog", async () => {
    api.listScreenDocs.mockResolvedValue([]);
    render(Page);
    const create = await screen.findByRole("button", { name: /Create your first screen/ });
    expect(create.classList.contains("vf")).toBe(true);
    expect(create.hasAttribute("data-selected")).toBe(true);
    await fireEvent.click(create);
    expect(await screen.findByRole("heading", { name: "New screen" })).toBeTruthy();
    expect(screen.getByLabelText("Name")).toHaveProperty("value", "Screen 1");
  });

  it("deletes after confirmation, and keeps the screen when it fails", async () => {
    render(Page);
    await waitFor(() => card("Main program"));
    await fireEvent.click(within(card("Main program")).getByRole("button", { name: "Delete Main program" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("Delete screen?");
    api.listScreenDocs.mockResolvedValue([vert]);
    await fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    expect(api.deleteScreen).toHaveBeenCalledWith("m1");
    await waitFor(() => expect(document.querySelectorAll("article[data-screen]")).toHaveLength(1));
    expect(card("Vertical").hasAttribute("data-selected")).toBe(true);

    api.deleteScreen.mockRejectedValueOnce({ code: "io", message: "Locked" });
    await fireEvent.click(within(card("Vertical")).getByRole("button", { name: "Delete Vertical" }));
    await fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(toasts.list.some((t) => /Locked/.test(t.message))).toBe(true));
    expect(document.querySelectorAll("article[data-screen]")).toHaveLength(1);
  });

  it("renames in place, and rolls back on failure", async () => {
    render(Page);
    await waitFor(() => card("Main program"));
    await fireEvent.click(within(card("Main program")).getByRole("button", { name: "Rename Main program" }));
    const input = screen.getByLabelText("Screen name") as HTMLInputElement;
    expect(input.value).toBe("Main program");
    await fireEvent.input(input, { target: { value: "Studio" } });
    await fireEvent.submit(input.closest("form")!);
    expect(api.renameScreen).toHaveBeenCalledWith("m1", "Studio");
    await waitFor(() => card("Studio"));

    api.renameScreen.mockRejectedValueOnce({ code: "invalid", message: "Name taken" });
    await fireEvent.click(within(card("Vertical")).getByRole("button", { name: "Rename Vertical" }));
    const again = screen.getByLabelText("Screen name") as HTMLInputElement;
    await fireEvent.input(again, { target: { value: "Upright" } });
    await fireEvent.submit(again.closest("form")!);
    await waitFor(() => expect(toasts.list.some((t) => /Name taken/.test(t.message))).toBe(true));
    // Rolled back; the input stays open for another try.
    expect(screen.getByLabelText("Screen name")).toBeInTheDocument();
    const v2 = document.querySelector("article[data-screen=\"v2\"]");
    expect(v2).toHaveTextContent("Vertical");
    expect(v2?.textContent).not.toContain("Upright");
  });

  it("duplicates and selects the copy", async () => {
    render(Page);
    await waitFor(() => card("Main program"));
    api.listScreenDocs.mockResolvedValue([vert, main, summary("c3", "Main program copy")]);
    await fireEvent.click(within(card("Main program")).getByRole("button", { name: "Duplicate Main program" }));
    expect(api.duplicateScreen).toHaveBeenCalledWith("m1", "Main program copy");
    await waitFor(() => expect(card("Main program copy").hasAttribute("data-selected")).toBe(true));
  });

  it("reloads when a screen file changes on disk, and unsubscribes on teardown", async () => {
    const { unmount } = render(Page);
    await waitFor(() => card("Main program"));
    expect(api.listScreenDocs).toHaveBeenCalledTimes(1);
    api.listScreenDocs.mockResolvedValue([main]);
    events.handlers.get("project-files-changed")?.({ kind: "template", id: "t", change: "changed", message: null });
    expect(api.listScreenDocs).toHaveBeenCalledTimes(1);
    events.handlers.get("project-files-changed")?.({ kind: "screen", id: "v2", change: "removed", message: null });
    await waitFor(() => expect(document.querySelectorAll("article[data-screen]")).toHaveLength(1));
    unmount();
    expect(events.handlers.has("project-files-changed")).toBe(false);
  });
});

// ---- New screen dialog --------------------------------------------------------------------------

describe("New screen dialog", () => {
  function open(names: string[] = ["Main", "Other"]) {
    const onclose = vi.fn();
    render(NewScreenDialog, { open: true, onclose, names });
    return onclose;
  }
  const createBtn = () => screen.getByRole("button", { name: "Create" });

  it("defaults: next free name, the project's resolution, transparent", async () => {
    open();
    expect(await screen.findByRole("heading", { name: "New screen" })).toBeTruthy();
    const name = screen.getByLabelText("Name") as HTMLInputElement;
    expect(name.value).toBe("Screen 3");
    expect(name.getAttribute("placeholder")).toBeNull();
    expect((screen.getByRole("radio", { name: /1280×720/ }) as HTMLInputElement).checked).toBe(true);
    expect((screen.getByRole("radio", { name: "Transparent" }) as HTMLInputElement).checked).toBe(true);
    expect(screen.getAllByRole("radio").map((r) => r.parentElement?.textContent?.trim())).toEqual([
      "1920×1080 Full HD",
      "1280×720 HD",
      "3840×2160 4K UHD",
      "1080×1920 Vertical",
      "Custom Any size",
      "Transparent",
      "Color",
    ]);
  });

  it("loads the existing names itself when none are given", async () => {
    api.listScreenDocs.mockResolvedValue([main, vert]);
    render(NewScreenDialog, { open: true, onclose: vi.fn() });
    await waitFor(() =>
      expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Screen 3"),
    );
  });

  it("validates a custom size within the range, inline", async () => {
    open();
    await fireEvent.click(screen.getByRole("radio", { name: /Custom/ }));
    const w = screen.getByLabelText("Width") as HTMLInputElement;
    const h = screen.getByLabelText("Height") as HTMLInputElement;
    expect(w.value).toBe("1280");
    await fireEvent.change(w, { target: { value: "10" } });
    expect(screen.getByText(/must be whole numbers from/)).toBeTruthy();
    expect(w).toHaveAttribute("aria-invalid", "true");
    expect(createBtn()).toBeDisabled();
    await fireEvent.change(w, { target: { value: "9000" } });
    expect(createBtn()).toBeDisabled();
    await fireEvent.change(w, { target: { value: "800" } });
    await fireEvent.change(h, { target: { value: "600" } });
    expect(screen.queryByText(/must be whole numbers from/)).toBeNull();
    expect(createBtn()).not.toBeDisabled();
    await fireEvent.click(createBtn());
    await waitFor(() =>
      expect(api.createScreen).toHaveBeenCalledWith({
        name: "Screen 3",
        width: 800,
        height: 600,
        background: "transparent",
      }),
    );
  });

  it("validates the colour hex, then creates and opens the editor", async () => {
    const onclose = open();
    await fireEvent.click(screen.getByRole("radio", { name: /3840×2160/ }));
    await fireEvent.click(screen.getByRole("radio", { name: "Color" }));
    const hex = screen.getByLabelText("Background color code") as HTMLInputElement;
    await fireEvent.input(hex, { target: { value: "#zz" } });
    expect(screen.getByText(/Enter a color like/)).toBeTruthy();
    expect(createBtn()).toBeDisabled();
    await fireEvent.input(hex, { target: { value: "#FF0000" } });
    expect(screen.queryByText(/Enter a color like/)).toBeNull();
    expect((screen.getByLabelText("Background color") as HTMLInputElement).value).toBe("#ff0000");
    const name = screen.getByLabelText("Name") as HTMLInputElement;
    await fireEvent.input(name, { target: { value: "  Big wall  " } });
    await fireEvent.click(createBtn());
    await waitFor(() => expect(nav.goto).toHaveBeenCalledWith("/screens/n1"));
    expect(api.createScreen).toHaveBeenCalledWith({
      name: "Big wall",
      width: 3840,
      height: 2160,
      background: "#ff0000",
    });
    expect(onclose).toHaveBeenCalled();
  });

  it("shows a failed create inline and stays open", async () => {
    const onclose = open();
    api.createScreen.mockRejectedValueOnce({ code: "io", message: "Disk full" });
    await fireEvent.click(createBtn());
    expect(await screen.findByRole("alert")).toHaveTextContent("Disk full");
    expect(onclose).not.toHaveBeenCalled();
    expect(nav.goto).not.toHaveBeenCalled();
    expect(screen.getByRole("heading", { name: "New screen" })).toBeTruthy();
  });

  it("an empty name can't be created; Escape closes", async () => {
    const onclose = open();
    const name = screen.getByLabelText("Name") as HTMLInputElement;
    await fireEvent.input(name, { target: { value: "   " } });
    expect(createBtn()).toBeDisabled();
    await fireEvent(screen.getByRole("dialog"), new Event("cancel", { cancelable: true }));
    expect(onclose).toHaveBeenCalledTimes(1);
  });
});

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  LiveScreenView,
  PresetInfo,
  Project,
  ServerStatus,
  TemplateSummary,
  UiState,
} from "$lib/types";

// ---- Mocks --------------------------------------------------------------------------------------

type Handler = (payload: unknown) => void;

const events = vi.hoisted(() => ({ handlers: new Map<string, Handler>() }));
const api = vi.hoisted(() => ({
  serverStatus: vi.fn(),
  listScreens: vi.fn(),
  liveShow: vi.fn(),
  liveHide: vi.fn(),
  listTemplates: vi.fn(),
  listPresets: vi.fn(),
  listStyles: vi.fn(() => Promise.resolve([])),
  getTemplate: vi.fn(),
  setDefaultTemplateVariant: vi.fn(),
  createTemplate: vi.fn(),
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
vi.mock("$lib/api/templates", () => api);

const nav = vi.hoisted(() => ({ goto: vi.fn() }));
vi.mock("$app/navigation", () => ({ goto: nav.goto, afterNavigate: vi.fn() }));
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/dashboard") } }));
vi.mock("$lib/projectFlow", () => ({ revealInExplorer: vi.fn() }));

// Observers: tests drive intersections by hand.
const obs = vi.hoisted(() => ({
  io: [] as { cb: IntersectionObserverCallback; targets: Element[]; off: boolean }[],
  ro: [] as { targets: Element[]; off: boolean }[],
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
  #rec: (typeof obs.ro)[number] = { targets: [], off: false };
  constructor(_cb: ResizeObserverCallback) {
    obs.ro.push(this.#rec);
  }
  observe(el: Element) {
    this.#rec.targets.push(el);
  }
  unobserve() {}
  disconnect() {
    this.#rec.off = true;
  }
}
vi.stubGlobal("IntersectionObserver", FakeIO);
vi.stubGlobal("ResizeObserver", FakeRO);

let visibility: DocumentVisibilityState = "visible";
Object.defineProperty(document, "visibilityState", {
  configurable: true,
  get: () => visibility,
});

/** Fires an intersection change for thumbnail boxes of the given screen ids (all if omitted). */
async function intersect(isIntersecting: boolean, ids?: string[]) {
  for (const rec of obs.io) {
    if (rec.off) continue;
    const targets = rec.targets.filter(
      (t) => !ids || ids.includes((t as HTMLElement).dataset.screenId ?? ""),
    );
    if (targets.length === 0) continue;
    const entries = targets.map((target) => ({ target, isIntersecting }));
    rec.cb(entries as unknown as IntersectionObserverEntry[], {} as IntersectionObserver);
  }
  await tick();
}

import DashboardPage from "../../../routes/(app)/dashboard/+page.svelte";
import RecentTemplates from "./RecentTemplates.svelte";
import ServerPanel from "./ServerPanel.svelte";
import {
  MAX_LIVE_THUMBNAILS,
  RECENT_TEMPLATES,
  isEmptyProject,
  pickLive,
  recentTemplates,
  screenTally,
  userScreens,
} from "./data";
import { project } from "$lib/stores/project.svelte";
import { live } from "$lib/stores/live.svelte";
import { server } from "$lib/stores/server.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { uiState } from "$lib/stores/uiState.svelte";

// ---- Fixtures -----------------------------------------------------------------------------------

const running = {
  state: "running",
  port: 7770,
  url: "http://127.0.0.1:7770",
  lanUrl: null,
} satisfies ServerStatus;

const current: Project = {
  path: "C:\\shows\\current",
  name: "Current Show",
  createdAt: "2026-01-02T10:00:00Z",
  settings: { port: 7770, lanAccess: false, defaultResolution: { width: 1920, height: 1080 } },
};

const baseUi: UiState = {
  sidebarPinned: false,
  control: {
    view: "grid",
    gridColumns: 2,
    selectedScreenId: null,
    hiddenScreenIds: [],
    liveEdit: "immediate",
    hotkeys: {},
    automationOff: false,
  },
};

function view(id: string, onAir = false): LiveScreenView {
  return {
    screen: { id, name: `Screen ${id}`, width: 1920, height: 1080, background: "transparent" },
    layers: [{ id: "l", name: "Layer", onAir, fields: {}, fieldDefs: [], pending: false }],
  };
}

const testScreen = view("test");

function tmpl(id: string, name: string, updatedAt: string, presetId?: string): TemplateSummary {
  return {
    id,
    name,
    description: "",
    preset: presetId ? { id: presetId, version: 1 } : null,
    fieldCount: 2,
    variableCount: 3,
    variantCount: 1,
    updatedAt,
    presetUpdate: null,
  };
}

const presets: PresetInfo[] = [
  { id: "classic-bar", name: "Classic bar", description: "", version: 1 },
  { id: "news-strap", name: "News strap", description: "", version: 1 },
];

beforeEach(() => {
  live.reset();
  vi.clearAllMocks();
  obs.io.length = 0;
  obs.ro.length = 0;
  events.handlers.clear();
  visibility = "visible";
  server.status = null;
  localStorage.clear();
  api.on.mockImplementation((name: string, h: Handler) => {
    events.handlers.set(name, h);
    return () => events.handlers.delete(name);
  });
  api.serverStatus.mockResolvedValue(running);
  api.listScreens.mockResolvedValue([testScreen]);
  api.listTemplates.mockResolvedValue([]);
  api.listPresets.mockResolvedValue(presets);
  project.current = current;
  uiState.set(baseUi);
});

afterEach(() => {
  cleanup();
  toasts.clear();
  vi.restoreAllMocks();
});

const frames = () => [...document.querySelectorAll("iframe")];

// ---- Rules --------------------------------------------------------------------------------------

describe("dashboard data rules", () => {
  it("leaves the dev test screen out of the user's screens", () => {
    expect(userScreens([testScreen, view("a"), view("b")]).map((s) => s.screen.id)).toEqual([
      "a",
      "b",
    ]);
  });

  it("is empty only without screens and templates", () => {
    expect(isEmptyProject([], [])).toBe(true);
    expect(isEmptyProject([view("a")], [])).toBe(false);
    expect(isEmptyProject([], [tmpl("t", "T", "2026-01-01T00:00:00Z")])).toBe(false);
  });

  it("recent templates: newest updatedAt first, top 5", () => {
    const list = [
      tmpl("a", "A", "2026-01-01T00:00:00Z"),
      tmpl("b", "B", "2026-03-01T00:00:00Z"),
      tmpl("c", "C", "2026-02-01T00:00:00Z"),
      tmpl("d", "D", "2026-05-01T00:00:00Z"),
      tmpl("e", "E", "2026-04-01T00:00:00Z"),
      tmpl("f", "F", "2026-06-01T00:00:00Z"),
      tmpl("g", "G", "not a date"),
    ];
    expect(RECENT_TEMPLATES).toBe(5);
    expect(recentTemplates(list).map((t) => t.id)).toEqual(["f", "d", "e", "b", "c"]);
    expect(recentTemplates(list, 2).map((t) => t.id)).toEqual(["f", "d"]);
    expect(recentTemplates([])).toEqual([]);
    // Not sorted in place.
    expect(list.map((t) => t.id).join("")).toBe("abcdefg");
  });

  it("tally: on air beats ready beats idle", () => {
    expect(screenTally(view("a", true), null)).toBe("program");
    expect(screenTally(view("a"), running)).toBe("preview");
    expect(screenTally(view("a"), { state: "stopped" })).toBe("idle");
    expect(screenTally(view("a"), null)).toBe("idle");
  });

  it("picks at most the cap, in grid order, from what is visible", () => {
    const order = ["a", "b", "c", "d", "e", "f", "g", "h"];
    expect([...pickLive(order, new Set(order))]).toEqual(["a", "b", "c", "d", "e", "f"]);
    expect([...pickLive(order, new Set(["h", "b"]))]).toEqual(["b", "h"]);
    expect([...pickLive(order, new Set(["a", "b", "c"]), 2)]).toEqual(["a", "b"]);
    expect(MAX_LIVE_THUMBNAILS).toBe(6);
  });
});

// ---- Empty project ------------------------------------------------------------------------------

describe("Dashboard, empty project", () => {
  it("shows two bracket-framed create actions, the server panel, and no placeholder copy", async () => {
    render(DashboardPage);
    const first = await screen.findByRole("button", { name: /Create your first screen/ });
    const create = screen.getByRole("button", { name: /Create your first template/ });
    expect(first.tagName).toBe("BUTTON"); // opens the New screen dialog
    for (const a of [first, create]) {
      expect(a.classList.contains("vf")).toBe(true);
      expect(a.hasAttribute("data-selected")).toBe(true);
    }
    // The dev test screen is not a user screen: no grid, no thumbnails.
    expect(screen.queryByRole("heading", { name: /^Screens/ })).toBeNull();
    expect(frames()).toHaveLength(0);
    expect(screen.getByRole("heading", { name: "Output server" })).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "Recent templates" })).toBeNull();
    expect(document.body.textContent).not.toMatch(/later milestone|coming|placeholder|sample/i);
  });

  it("renders neither state until the screen list has loaded", async () => {
    let resolve: (v: LiveScreenView[]) => void = () => {};
    api.listScreens.mockReturnValue(new Promise((r) => (resolve = r)));
    render(DashboardPage);
    await tick();
    expect(screen.queryByRole("button", { name: /Create your first screen/ })).toBeNull();
    resolve([]);
    expect(await screen.findByRole("button", { name: /Create your first screen/ })).toBeTruthy();
  });

  it("waits for the template list too", async () => {
    let resolve: (v: TemplateSummary[]) => void = () => {};
    api.listTemplates.mockReturnValue(new Promise((r) => (resolve = r)));
    render(DashboardPage);
    await tick();
    await tick();
    expect(screen.queryByRole("button", { name: /Create your first screen/ })).toBeNull();
    resolve([]);
    expect(await screen.findByRole("button", { name: /Create your first screen/ })).toBeTruthy();
  });

  it("Create your first template opens the New template gallery (lazy)", async () => {
    render(DashboardPage);
    const create = await screen.findByRole("button", { name: /Create your first template/ });
    expect(screen.queryByRole("heading", { name: "New template" })).toBeNull();
    await fireEvent.click(create);
    expect(await screen.findByRole("heading", { name: "New template" })).toBeTruthy();
    expect(nav.goto).not.toHaveBeenCalled();
  });
});

// ---- Templates but no screens -------------------------------------------------------------------

describe("Dashboard with templates", () => {
  it("lists recent templates (newest first, origin, relative time in body text) and offers the first screen", async () => {
    const recent = new Date(Date.now() - 5 * 60_000).toISOString();
    api.listTemplates.mockResolvedValue([
      tmpl("old", "Old strap", "2026-01-01T00:00:00Z", "news-strap"),
      tmpl("new", "Match bar", recent, "classic-bar"),
      tmpl("mid", "Own design", "2026-02-01T00:00:00Z"),
      tmpl("gone", "Orphan", "2025-12-01T00:00:00Z", "retired-preset"),
    ]);
    render(DashboardPage);
    const panel = (await screen.findByRole("heading", { name: "Recent templates" })).closest(
      "section",
    ) as HTMLElement;
    const links = within(panel).getAllByRole("link");
    expect(links.map((a) => a.getAttribute("href"))).toEqual([
      "/templates/new",
      "/templates/mid",
      "/templates/old",
      "/templates/gone",
    ]);
    expect(links[0].textContent).toContain("Match bar");
    expect(links[0].textContent).toContain("From Classic bar");
    expect(links[1].textContent).toContain("Blank");
    expect(links[2].textContent).toContain("From News strap");
    expect(links[3].textContent).toContain("From retired-preset");
    const time = links[0].querySelector("time");
    expect(time?.classList.contains("mono")).toBe(false);
    expect(time?.textContent).toMatch(/minutes ago/);

    // No screens yet: the screens block offers the first one; no template create tile.
    expect(screen.getByRole("heading", { name: /^Screens/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Create your first screen/ })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Create your first template/ })).toBeNull();
    expect(frames()).toHaveLength(0);
  });

  it("shows at most five", async () => {
    api.listTemplates.mockResolvedValue(
      Array.from({ length: 8 }, (_, i) => tmpl(`t${i}`, `T${i}`, `2026-0${i + 1}-01T00:00:00Z`)),
    );
    render(DashboardPage);
    const panel = (await screen.findByRole("heading", { name: "Recent templates" })).closest(
      "section",
    ) as HTMLElement;
    const links = within(panel).getAllByRole("link");
    expect(links).toHaveLength(5);
    expect(links[0].getAttribute("href")).toBe("/templates/t7");
  });

  it("a failed template list still renders the dashboard, with a toast", async () => {
    api.listTemplates.mockRejectedValue({ code: "io", message: "disk gone" });
    render(DashboardPage);
    expect(await screen.findByRole("button", { name: /Create your first screen/ })).toBeTruthy();
    expect(toasts.list.some((t) => t.message.includes("disk gone"))).toBe(true);
  });

  it("Create a template in the panel opens the gallery", async () => {
    api.listScreens.mockResolvedValue([testScreen, view("s1")]);
    render(DashboardPage);
    const create = await screen.findByRole("button", { name: "Create a template" });
    await fireEvent.click(create);
    expect(await screen.findByRole("heading", { name: "New template" })).toBeTruthy();
  });
});

// ---- Screens grid -------------------------------------------------------------------------------

describe("Dashboard screens grid", () => {
  const many = (n: number) => Array.from({ length: n }, (_, i) => view(`s${i + 1}`, i === 0));

  async function mountWith(list: LiveScreenView[]) {
    api.listScreens.mockResolvedValue([testScreen, ...list]);
    render(DashboardPage);
    await screen.findByRole("heading", { name: /^Screens/ });
    await waitFor(() => expect(server.status).toEqual(running));
  }

  it("lists the user's screens with tally, mono URL and copy, without the test screen", async () => {
    await mountWith(many(2));
    const cards = screen.getAllByRole("article");
    expect(cards).toHaveLength(2);
    const first = within(cards[0]);
    expect(first.getByRole("link", { name: "Screen s1" }).getAttribute("href")).toBe("/screens/s1");
    expect(first.getByRole("img", { name: "On air" })).toBeTruthy();
    expect(within(cards[1]).getByRole("img", { name: "Ready" })).toBeTruthy();
    expect(first.getByText("http://127.0.0.1:7770/screen/s1").classList.contains("mono")).toBe(
      true,
    );
    expect(first.getByRole("button", { name: "Copy URL of Screen s1" })).toBeTruthy();
    expect(screen.queryByText("Screen test")).toBeNull();
    expect(screen.queryByRole("link", { name: /Create your first/ })).toBeNull();
    expect(screen.getByRole("heading", { name: "Recent templates" })).toBeTruthy();
  });

  it("marks the control-selected screen (a live output: no accent mark)", async () => {
    uiState.set({ ...baseUi, control: { ...baseUi.control, selectedScreenId: "s2" } });
    await mountWith(many(2));
    const cards = screen.getAllByRole("article");
    expect(cards[0].hasAttribute("data-selected")).toBe(false);
    expect(cards[1].hasAttribute("data-selected")).toBe(true);
    expect(cards[1].classList.contains("vf")).toBe(true);
    // A live output: vf-live (no accent selection border; a neutral one from the card).
    expect(cards[1].classList.contains("vf-live")).toBe(true);
  });

  it("uses one IntersectionObserver and one ResizeObserver for the grid", async () => {
    await mountWith(many(4));
    expect(obs.io.filter((r) => !r.off)).toHaveLength(1);
    expect(obs.ro.filter((r) => !r.off)).toHaveLength(1);
    expect(obs.io[0].targets).toHaveLength(4);
  });

  it("mounts a thumbnail only while intersecting, and removes it after", async () => {
    await mountWith(many(3));
    expect(frames()).toHaveLength(0);
    await intersect(true, ["s2"]);
    expect(frames().map((f) => f.getAttribute("src"))).toEqual([
      "http://127.0.0.1:7770/screen/s2?mute=1",
    ]);
    const f = frames()[0];
    expect(f.getAttribute("tabindex")).toBe("-1");
    expect(f.getAttribute("sandbox")).toBe("allow-scripts allow-same-origin");
    await intersect(false, ["s2"]);
    expect(frames()).toHaveLength(0);
  });

  it("caps live thumbnails at 6; the rest say Preview paused", async () => {
    await mountWith(many(8));
    await intersect(true);
    expect(frames()).toHaveLength(6);
    expect(screen.getAllByText("Preview paused")).toHaveLength(2);
    // Scrolling the first ones away lets the paused ones in.
    await intersect(false, ["s1", "s2"]);
    expect(frames().map((f) => f.getAttribute("src")?.split("/").pop()?.replace("?mute=1", ""))).toEqual([
      "s3",
      "s4",
      "s5",
      "s6",
      "s7",
      "s8",
    ]);
    expect(screen.queryByText("Preview paused")).toBeNull();
  });

  it("removes thumbnails while the window is hidden or the server is down", async () => {
    await mountWith(many(2));
    await intersect(true);
    expect(frames()).toHaveLength(2);

    visibility = "hidden";
    document.dispatchEvent(new Event("visibilitychange"));
    await tick();
    expect(frames()).toHaveLength(0);
    visibility = "visible";
    document.dispatchEvent(new Event("visibilitychange"));
    await tick();
    expect(frames()).toHaveLength(2);

    events.handlers.get("server-status")?.({ state: "stopped" });
    await tick();
    expect(frames()).toHaveLength(0);
    expect(screen.getAllByText("Server not running")).toHaveLength(2);
  });

  it("disconnects its observers on unmount", async () => {
    await mountWith(many(2));
    cleanup();
    expect(obs.io.every((r) => r.off)).toBe(true);
    expect(obs.ro.every((r) => r.off)).toBe(true);
  });
});

// ---- Panels -------------------------------------------------------------------------------------

describe("ServerPanel", () => {
  it("shows status, mono local URL with copy, LAN and port, and links to Preferences", () => {
    project.current = { ...current, settings: { ...current.settings, lanAccess: true } };
    render(ServerPanel, {
      status: { ...running, lanUrl: "http://192.168.1.20:7770" },
    });
    expect(screen.getByRole("status").textContent).toContain("Running");
    expect(screen.getByText("127.0.0.1:7770").classList.contains("mono")).toBe(true);
    expect(screen.getByText("192.168.1.20:7770").classList.contains("mono")).toBe(true);
    expect(screen.getByRole("button", { name: "Copy local URL" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Copy LAN URL" })).toBeTruthy();
    expect(screen.getByText("7770").classList.contains("mono")).toBe(true);
    expect(screen.getByRole("link", { name: /Server preferences/ }).getAttribute("href")).toBe(
      "/settings#server",
    );
  });

  it("says Off for LAN and shows no URL while stopped", () => {
    render(ServerPanel, { status: { state: "stopped" } });
    expect(screen.getByRole("status").textContent).toContain("Stopped");
    expect(screen.getByText("Off")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Copy/ })).toBeNull();
  });
});

describe("RecentTemplates", () => {
  it("links each row to its editor, in the given order, or offers to create one", async () => {
    const oncreate = vi.fn();
    render(RecentTemplates, {
      templates: [
        tmpl("a", "Lower third", new Date().toISOString(), "classic-bar"),
        tmpl("b", "Strap", "2026-01-01T00:00:00Z"),
      ],
      presets,
      oncreate,
    });
    const links = screen.getAllByRole("link");
    expect(links.map((a) => a.getAttribute("href"))).toEqual(["/templates/a", "/templates/b"]);
    expect(links[0].querySelector(".name")?.textContent).toBe("Lower third");
    expect(links[0].querySelector(".origin")?.textContent).toBe("From Classic bar");
    expect(links[0].querySelector("time")?.textContent).toBe("just now");
    expect(links[1].querySelector(".origin")?.textContent).toBe("Blank");
    cleanup();

    render(RecentTemplates, { templates: [], presets, oncreate });
    expect(screen.getByText("No templates yet.")).toBeTruthy();
    expect(screen.queryByRole("link")).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Create a template" }));
    expect(oncreate).toHaveBeenCalledOnce();
  });
});

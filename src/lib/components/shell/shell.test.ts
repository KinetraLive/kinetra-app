import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { tick } from "svelte";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { Project, RecentProject, ServerStatus, UiState } from "$lib/types";

const api = vi.hoisted(() => ({
  listRecents: vi.fn(),
  openProject: vi.fn(),
  closeProject: vi.fn(),
  currentProject: vi.fn(),
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  getPrefs: vi.fn(),
  setPrefs: vi.fn(),
  serverStatus: vi.fn(),
  on: vi.fn(),
  openPreferences: vi.fn(async (_section?: string) => {}),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) => ({ code: "internal", message: String(e) }),
}));
// Domain modules share the same mock functions (the split only changes where they are imported from).
vi.mock("$lib/api/live", () => api);


const nav = vi.hoisted(() => ({
  goto: vi.fn(async (_url: string) => {}),
  afterNavigate: vi.fn(),
}));
vi.mock("$app/navigation", () => nav);

const pageState = vi.hoisted(() => ({ page: { url: new URL("http://localhost/templates") } }));
vi.mock("$app/state", () => pageState);

vi.mock("$lib/projectFlow", () => ({
  pickFolder: vi.fn(),
  openFolder: vi.fn(),
  revealInExplorer: vi.fn(),
  describeEntries: (n: number) => `${n} items`,
}));

import AppShell from "./AppShell.svelte";
import ProjectSwitcher from "./ProjectSwitcher.svelte";
import Sidebar, { HOVER_CLOSE_MS, HOVER_OPEN_MS } from "./Sidebar.svelte";
import TopBar from "./TopBar.svelte";
import { server } from "$lib/stores/server.svelte";
import { project } from "$lib/stores/project.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { uiState } from "$lib/stores/uiState.svelte";

const current: Project = {
  path: "C:\\shows\\current",
  name: "Current Show",
  createdAt: "2026-01-02T10:00:00Z",
  settings: { port: 7770, lanAccess: false, defaultResolution: { width: 1920, height: 1080 } },
};

const recents: RecentProject[] = [
  { path: current.path, name: current.name, lastOpenedAt: "2026-10-01T00:00:00Z", missing: false },
  { path: "C:\\shows\\other", name: "Other Show", lastOpenedAt: "2026-09-01T00:00:00Z", missing: false },
  { path: "C:\\shows\\gone", name: "Gone Show", lastOpenedAt: "2026-08-01T00:00:00Z", missing: true },
];

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

let unsubscribe = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  unsubscribe = vi.fn();
  api.on.mockImplementation(() => unsubscribe);
  api.serverStatus.mockResolvedValue({ state: "stopped" });
  api.listRecents.mockResolvedValue(recents);
  api.closeProject.mockResolvedValue(undefined);
  api.setUiState.mockImplementation(async (v: UiState) => v);
  pageState.page.url = new URL("http://localhost/templates");
  project.current = current;
  project.recents = [];
  uiState.set(baseUi);
});

afterEach(() => {
  cleanup();
  toasts.clear();
  vi.restoreAllMocks();
});

describe("Logo", () => {
  it("links to the dashboard as Kinetra, with the brand logo for each theme", () => {
    render(TopBar);
    const logo = screen.getByRole("link", { name: "Kinetra" });
    expect(logo.getAttribute("href")).toBe("/dashboard");
    const imgs = [...logo.querySelectorAll("img")];
    expect(imgs.map((i) => i.className.split(" ")[0])).toEqual(["for-dark", "for-light"]);
    expect(imgs.every((i) => i.getAttribute("height") === "32")).toBe(true);
    // A stable build: no channel badge.
    expect(logo.textContent?.toLowerCase()).not.toContain("canary");
  });
});

describe("Sidebar", () => {
  const overlay = () => document.querySelector<HTMLElement>(".overlay");
  const isOpen = () => overlay()?.classList.contains("open") ?? false;

  it("marks the active route with aria-current and an accent dot, not brackets", () => {
    render(Sidebar);
    expect(screen.getByRole("navigation", { name: "Main" })).toBeTruthy();
    const templates = screen.getByRole("link", { name: "Templates" });
    expect(templates.getAttribute("aria-current")).toBe("page");
    // The dot marks the active item; brackets (vf) are only drawn on keyboard focus.
    expect(templates.querySelector(".current-dot")?.getAttribute("aria-hidden")).toBe("true");
    expect(templates.classList.contains("vf")).toBe(true);
    expect(templates.hasAttribute("data-selected")).toBe(false);
    const dashboard = screen.getByRole("link", { name: "Dashboard" });
    expect(dashboard.getAttribute("aria-current")).toBeNull();
    expect(dashboard.querySelector(".current-dot")).toBeNull();
    const prefs = screen.getByRole("link", { name: "Preferences" });
    expect(prefs.getAttribute("href")).toBe("/settings");
    expect(prefs.getAttribute("aria-current")).toBeNull();
  });

  it("Preferences opens the Preferences window instead of navigating", async () => {
    render(Sidebar);
    const prefs = screen.getByRole("link", { name: "Preferences" });
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    prefs.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
    expect(api.openPreferences).toHaveBeenCalledWith(undefined);
  });

  it("marks nested routes as inside their section", () => {
    pageState.page.url = new URL("http://localhost/settings");
    render(Sidebar);
    expect(screen.getByRole("link", { name: "Preferences" }).getAttribute("aria-current")).toBe(
      "page",
    );
  });

  it("the pin button toggles sidebarPinned through uiState.update", async () => {
    const update = vi.spyOn(uiState, "update");
    render(Sidebar);
    const pin = screen.getByRole("button", { name: "Pin sidebar" });
    expect(pin.getAttribute("aria-pressed")).toBe("false");
    // Neutral icon button: the state is in aria-pressed and the icon, never an accent fill.
    expect(pin.classList.contains("fill")).toBe(false);
    await fireEvent.click(pin);
    expect(update).toHaveBeenCalledWith({ ...baseUi, sidebarPinned: true });
    expect(api.setUiState).toHaveBeenCalledWith({ ...baseUi, sidebarPinned: true });
    await tick();
    const unpin = screen.getByRole("button", { name: "Unpin sidebar" });
    expect(unpin.getAttribute("aria-pressed")).toBe("true");
    // Pinned: labelled rows in the layout, no overlay.
    expect(document.querySelector(".sidebar")?.getAttribute("data-mode")).toBe("pinned");
    expect(overlay()).toBeNull();
    expect(screen.getByRole("link", { name: "Templates" }).textContent).toContain("Templates");

    await fireEvent.click(unpin);
    expect(update).toHaveBeenLastCalledWith({ ...baseUi, sidebarPinned: false });
  });

  it("rolls back and toasts when saving the pin fails", async () => {
    api.setUiState.mockRejectedValue("disk full");
    render(Sidebar);
    await fireEvent.click(screen.getByRole("button", { name: "Pin sidebar" }));
    await vi.waitFor(() => expect(toasts.list.length).toBe(1));
    expect(uiState.value?.sidebarPinned).toBe(false);
    expect(toasts.list[0].kind).toBe("error");
    expect(toasts.list[0].message).toContain("disk full");
    await tick();
    expect(screen.getByRole("button", { name: "Pin sidebar" })).toBeTruthy();
  });

  it("moves focus between items with the arrow keys", async () => {
    uiState.set({ ...baseUi, sidebarPinned: true });
    render(Sidebar);
    const dash = screen.getByRole("link", { name: "Dashboard" });
    dash.focus();
    await fireEvent.keyDown(dash, { key: "ArrowDown" });
    expect(document.activeElement).toBe(screen.getByRole("link", { name: "Templates" }));
    await fireEvent.keyDown(document.activeElement as HTMLElement, { key: "End" });
    expect(document.activeElement).toBe(screen.getByRole("link", { name: "Preferences" }));
  });

  describe("hover overlay (unpinned)", () => {
    beforeEach(() => {
      vi.useFakeTimers();
    });
    afterEach(() => {
      vi.useRealTimers();
    });

    it("opens after the hover delay, closes on mouseleave; the grid column stays the rail", async () => {
      render(AppShell);
      const shell = document.querySelector(".shell") as HTMLElement;
      const sidebar = document.querySelector(".sidebar") as HTMLElement;
      expect(shell.dataset.sidebar).toBe("rail");
      expect(overlay()?.getAttribute("aria-hidden")).toBe("true");
      expect(overlay()?.inert).toBe(true);

      await fireEvent.mouseEnter(sidebar);
      await vi.advanceTimersByTimeAsync(HOVER_OPEN_MS - 20);
      expect(isOpen()).toBe(false);
      await vi.advanceTimersByTimeAsync(20);
      expect(isOpen()).toBe(true);
      expect(overlay()?.hasAttribute("aria-hidden")).toBe(false);
      expect(overlay()?.inert).toBe(false);
      expect(screen.getByRole("link", { name: "Templates" }).textContent).toContain("Templates");
      // The overlay floats over the content: the grid column (and the pin state) never change.
      expect(shell.dataset.sidebar).toBe("rail");
      expect(sidebar.dataset.mode).toBe("rail");
      expect(api.setUiState).not.toHaveBeenCalled();

      await fireEvent.mouseLeave(sidebar);
      expect(isOpen()).toBe(true); // grace period
      await vi.advanceTimersByTimeAsync(HOVER_CLOSE_MS);
      expect(isOpen()).toBe(false);
      expect(shell.dataset.sidebar).toBe("rail");
      expect(vi.getTimerCount()).toBe(0);
    });

    it("does not open when the pointer leaves before the delay", async () => {
      render(Sidebar);
      const sidebar = document.querySelector(".sidebar") as HTMLElement;
      await fireEvent.mouseEnter(sidebar);
      await vi.advanceTimersByTimeAsync(HOVER_OPEN_MS / 2);
      await fireEvent.mouseLeave(sidebar);
      await vi.advanceTimersByTimeAsync(500);
      expect(isOpen()).toBe(false);
      expect(vi.getTimerCount()).toBe(0);
    });

    it("a press outside collapses the open overlay at once; a press inside keeps it", async () => {
      render(AppShell);
      const sidebar = document.querySelector(".sidebar") as HTMLElement;
      await fireEvent.mouseEnter(sidebar);
      await vi.advanceTimersByTimeAsync(HOVER_OPEN_MS);
      expect(isOpen()).toBe(true);
      // The pointer is still over the overlay (it covers the content's left edge).
      await fireEvent.pointerDown(overlay() as HTMLElement);
      expect(isOpen()).toBe(true);
      await fireEvent.pointerDown(document.querySelector(".shell main, .shell .content") ?? document.body);
      expect(isOpen()).toBe(false);
    });

    it("runs no timers at rest and none survive unmount", async () => {
      const { unmount } = render(Sidebar);
      expect(vi.getTimerCount()).toBe(0);
      await fireEvent.mouseEnter(document.querySelector(".sidebar") as HTMLElement);
      // Let Svelte's own 0 ms event-propagation timer run; what remains is the hover delay.
      await vi.advanceTimersByTimeAsync(1);
      expect(vi.getTimerCount()).toBe(1);
      unmount();
      expect(vi.getTimerCount()).toBe(0);
    });
  });

  it("keyboard focus entering the rail opens the overlay; Escape closes it and restores focus", async () => {
    render(Sidebar);
    const railDash = document.querySelector<HTMLElement>('.rail [data-idx="0"]') as HTMLElement;
    railDash.focus();
    await vi.waitFor(() => expect(isOpen()).toBe(true));
    const overlayDash = overlay()?.querySelector<HTMLElement>('[data-idx="0"]') as HTMLElement;
    await vi.waitFor(() => expect(document.activeElement).toBe(overlayDash));
    expect(overlayDash.textContent).toContain("Dashboard");
    expect(document.querySelector(".rail")?.getAttribute("aria-hidden")).toBe("true");

    await fireEvent.keyDown(overlayDash, { key: "Escape" });
    await vi.waitFor(() => expect(document.activeElement).toBe(railDash));
    expect(isOpen()).toBe(false);
  });

  it("a pointer press does not open the overlay through focus", async () => {
    render(Sidebar);
    const railDash = document.querySelector<HTMLElement>('.rail [data-idx="0"]') as HTMLElement;
    await fireEvent.pointerDown(railDash);
    railDash.focus();
    await tick();
    expect(isOpen()).toBe(false);
  });

  it("the rail clips horizontally and has no pseudo-element labels", () => {
    // Component CSS isn't applied under jsdom, so check the rail's own rule in the source.
    const src = readFileSync(join(process.cwd(), "src/lib/components/shell/Sidebar.svelte"), "utf8");
    const rule = /\n {2}\.rail \{([^}]*)\}/.exec(src)?.[1] ?? "";
    expect(rule).toMatch(/overflow-x:\s*(hidden|clip)/);
    expect(src).not.toMatch(/attr\(data-tip\)/);
    render(Sidebar);
    const rail = document.querySelector(".rail") as HTMLElement;
    expect(rail.scrollWidth).toBeLessThanOrEqual(rail.clientWidth);
    // No tooltip-bearing elements (IconButton's data-tip) inside the rail.
    expect(rail.querySelector("[data-tip]")).toBeNull();
  });
});

describe("AppShell page transition", () => {
  it("can't change the scroll area or the content width while a page slides in", () => {
    // Regression: the entry slide moved a min-height:100% box down by --page-shift, a scrollbar
    // appeared for the animation's duration and the whole page shook sideways on every click.
    const src = readFileSync(join(process.cwd(), "src/lib/components/shell/AppShell.svelte"), "utf8");
    const rule = (sel: string) => new RegExp(`\\n {2}${sel} \\{([^}]*)\\}`).exec(src)?.[1] ?? "";
    expect(rule("\\.content")).toMatch(/scrollbar-gutter:\s*stable/);
    expect(rule("\\.page")).toMatch(/overflow-y:\s*clip/);
    // The animated element is the inner wrapper, inside the clipping one.
    expect(src).toMatch(/<div class="page">\s*(<!--[\s\S]*?-->\s*)?<div class="page-inner" bind:this=\{pageEl\}>/);
  });
});

describe("ProjectSwitcher", () => {
  async function openMenu() {
    render(ProjectSwitcher);
    const trigger = screen.getByRole("button", { name: /Current Show/ });
    await fireEvent.click(trigger);
    await vi.waitFor(() => expect(screen.getByRole("menuitem", { name: /Other Show/ })).toBeTruthy());
    return trigger;
  }

  it("lists recents except the current project, missing ones disabled", async () => {
    const trigger = await openMenu();
    expect(trigger.getAttribute("aria-expanded")).toBe("true");
    expect(trigger.getAttribute("aria-haspopup")).toBe("menu");
    expect(screen.queryByRole("menuitem", { name: /Current Show/ })).toBeNull();
    const gone = screen.getByRole("menuitem", { name: /Gone Show/ });
    expect(gone.getAttribute("aria-disabled")).toBe("true");
    expect(gone.textContent).toContain("Missing");
    expect(screen.getByRole("menuitem", { name: /New project/ })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: /Open/ })).toBeTruthy();
  });

  it("marks the current project with a selected tally dot; items get brackets", async () => {
    await openMenu();
    const currentRow = document.querySelector(".menu .current") as HTMLElement;
    expect(currentRow.textContent).toContain("Current Show");
    expect(currentRow.querySelector(".tally.selected")).toBeTruthy();
    for (const item of screen.getAllByRole("menuitem")) {
      expect(item.classList.contains("vf")).toBe(true);
    }
  });

  it("Esc closes the menu and returns focus to the trigger", async () => {
    const trigger = await openMenu();
    const first = screen.getAllByRole("menuitem")[0];
    first.focus();
    await fireEvent.keyDown(first, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    expect(document.activeElement).toBe(trigger);
  });

  it("Close project closes and navigates to /", async () => {
    await openMenu();
    await fireEvent.click(screen.getByRole("menuitem", { name: /Close project/ }));
    await vi.waitFor(() => expect(nav.goto).toHaveBeenCalledWith("/"));
    expect(api.closeProject).toHaveBeenCalledTimes(1);
    expect(project.current).toBeNull();
  });

  it("subscribes to recents-checked only while open", async () => {
    render(ProjectSwitcher);
    expect(api.on).not.toHaveBeenCalled();
    const trigger = screen.getByRole("button", { name: /Current Show/ });
    await fireEvent.click(trigger);
    await tick();
    expect(api.on).toHaveBeenCalledTimes(1);
    expect(api.on.mock.calls[0][0]).toBe("recents-checked");
    expect(unsubscribe).not.toHaveBeenCalled();

    await fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    await tick();
    expect(unsubscribe).toHaveBeenCalledTimes(1);
  });

  it("closes on a click outside", async () => {
    await openMenu();
    await fireEvent.pointerDown(document.body);
    await tick();
    expect(screen.queryByRole("menu")).toBeNull();
    expect(unsubscribe).toHaveBeenCalledTimes(1);
  });
});

describe("AppShell keyboard shortcuts", () => {
  it("Ctrl+, opens the Preferences window and Ctrl+2 goes to /templates", async () => {
    render(AppShell);
    await fireEvent.keyDown(window, { key: ",", code: "Comma", ctrlKey: true });
    expect(api.openPreferences).toHaveBeenCalledTimes(1);
    expect(nav.goto).not.toHaveBeenCalled();
    await fireEvent.keyDown(window, { key: "2", code: "Digit2", ctrlKey: true });
    expect(nav.goto).toHaveBeenCalledWith("/templates");
  });

  it("ignores Ctrl+2 while typing in an input, but not Ctrl+,", async () => {
    render(AppShell);
    const input = document.createElement("input");
    document.body.appendChild(input);
    try {
      input.focus();
      await fireEvent.keyDown(input, { key: "2", code: "Digit2", ctrlKey: true });
      expect(nav.goto).not.toHaveBeenCalled();
      await fireEvent.keyDown(input, { key: ",", code: "Comma", ctrlKey: true });
      expect(api.openPreferences).toHaveBeenCalledTimes(1);
    } finally {
      input.remove();
    }
  });

  it("Ctrl+B toggles the pin", async () => {
    render(AppShell);
    await fireEvent.keyDown(window, { key: "b", code: "KeyB", ctrlKey: true });
    expect(api.setUiState).toHaveBeenCalledWith({ ...baseUi, sidebarPinned: true });
    await tick();
    expect(document.querySelector(".shell")?.getAttribute("data-sidebar")).toBe("pinned");
    await fireEvent.keyDown(window, { key: "b", code: "KeyB", ctrlKey: true });
    expect(api.setUiState).toHaveBeenLastCalledWith({ ...baseUi, sidebarPinned: false });
  });

  it("Ctrl+B rolls back and toasts when saving fails", async () => {
    api.setUiState.mockRejectedValue("disk full");
    render(AppShell);
    await fireEvent.keyDown(window, { key: "b", code: "KeyB", ctrlKey: true });
    await vi.waitFor(() => expect(toasts.list.length).toBe(1));
    expect(uiState.value?.sidebarPinned).toBe(false);
    await tick();
    expect(document.querySelector(".shell")?.getAttribute("data-sidebar")).toBe("rail");
  });

  it("wraps the page in the animated content area", () => {
    render(AppShell);
    expect(document.querySelector("main.content > .page")).toBeTruthy();
  });
});

describe("command palette", () => {
  const palette = () => screen.queryByRole("dialog", { name: "Command palette" });
  const options = () => screen.getAllByRole("option");

  // The palette is a lazy chunk. Its first import compiles Palette.svelte and its modules in
  // this worker (~250 ms idle, 1.3–2.7 s with the machine busy), longer than `open()`'s
  // `waitFor` allows: load it once up front, as the editor tests do with their chunks. The
  // shell still opens it through its own dynamic import.
  beforeAll(async () => {
    await import("$lib/commands/palette/Palette.svelte");
  });

  async function open(init: KeyboardEventInit = { key: "k", code: "KeyK", ctrlKey: true }) {
    await fireEvent.keyDown(window, init);
    await vi.waitFor(() => expect(palette()).toBeTruthy());
    await tick();
  }

  it("Ctrl+K loads and opens it: the shell's commands with their shortcuts in mono", async () => {
    render(AppShell);
    expect(palette()).toBeNull();
    await open();
    const titles = options().map((o) => o.querySelector(".title")?.textContent);
    expect(titles).toEqual(expect.arrayContaining(["Go to Dashboard", "Go to Templates", "Toggle light / dark theme"]));
    const templates = options().find((o) => o.textContent?.includes("Go to Templates"))!;
    const kbd = templates.querySelector("kbd")!;
    expect(kbd.textContent).toBe("Ctrl+2");
    expect(kbd.classList.contains("mono")).toBe(true);
    // The search field has focus and the first option is active.
    expect(document.activeElement).toBe(screen.getByRole("combobox", { name: "Search commands" }));
    expect(options()[0].getAttribute("aria-selected")).toBe("true");
  });

  it("filters fuzzily; arrows move; Enter runs the command and closes", async () => {
    render(AppShell);
    await open();
    const input = screen.getByRole("combobox", { name: "Search commands" });
    await fireEvent.input(input, { target: { value: "prefer" } });
    expect(options().some((o) => o.textContent?.includes("Preferences…"))).toBe(true);
    await fireEvent.input(input, { target: { value: "gotmp" } });
    expect(options()[0].textContent).toContain("Go to Templates");
    await fireEvent.input(input, { target: { value: "go to" } });
    await fireEvent.keyDown(input, { key: "ArrowDown" });
    expect(options()[1].getAttribute("aria-selected")).toBe("true");
    expect(input.getAttribute("aria-activedescendant")).toBe(options()[1].id);
    await fireEvent.input(input, { target: { value: "go dash" } });
    expect(options()[0].getAttribute("aria-selected")).toBe("true");
    await fireEvent.keyDown(input, { key: "Enter" });
    await vi.waitFor(() => expect(nav.goto).toHaveBeenCalledWith("/dashboard"));
    await tick();
    expect(palette()).toBeNull();
  });

  it("Esc closes; Ctrl+Shift+P opens it too, even while typing; nothing matches says so", async () => {
    render(AppShell);
    await open();
    await fireEvent.keyDown(screen.getByRole("combobox", { name: "Search commands" }), { key: "Escape" });
    await tick();
    expect(palette()).toBeNull();
    const field = document.createElement("input");
    document.body.appendChild(field);
    try {
      field.focus();
      await fireEvent.keyDown(field, { key: "P", code: "KeyP", ctrlKey: true, shiftKey: true });
      await vi.waitFor(() => expect(palette()).toBeTruthy());
    } finally {
      field.remove();
    }
    await tick();
    await fireEvent.input(screen.getByRole("combobox", { name: "Search commands" }), { target: { value: "zzqx" } });
    expect(screen.getByText("No matches")).toBeTruthy();
  });

  it("lists commands registered elsewhere; disabled ones are shown but don't run", async () => {
    const { registerCommands } = await import("$lib/commands/registry.svelte");
    const run = vi.fn();
    const off = registerCommands([
      { id: "test.on", title: "Toggle grid", section: "settings", defaultKeys: ["Ctrl+G"], run },
      { id: "test.off", title: "Distribute horizontally", section: "settings", run, disabled: () => "Select three or more layers" },
    ]);
    try {
      render(AppShell);
      // A registered shortcut runs from the shell's key handler.
      await fireEvent.keyDown(window, { key: "g", code: "KeyG", ctrlKey: true });
      expect(run).toHaveBeenCalledTimes(1);
      await open();
      const input = screen.getByRole("combobox", { name: "Search commands" });
      await fireEvent.input(input, { target: { value: "distribute" } });
      expect(options()[0].getAttribute("aria-disabled")).toBe("true");
      await fireEvent.keyDown(input, { key: "Enter" });
      expect(run).toHaveBeenCalledTimes(1);
      expect(palette()).toBeTruthy();
      await fireEvent.input(input, { target: { value: "grid" } });
      await fireEvent.click(options()[0]);
      expect(run).toHaveBeenCalledTimes(2);
    } finally {
      off();
    }
  });
});

describe("sectionRoot", () => {
  it("keeps section roots and drops project-specific ids", async () => {
    const { sectionRoot } = await import("./nav");
    // Preferences is a window now: /settings only redirects.
    expect(sectionRoot("/settings")).toBe("/dashboard");
    expect(sectionRoot("/templates/abc123")).toBe("/templates");
    expect(sectionRoot("/templatesx")).toBe("/dashboard");
    expect(sectionRoot("/")).toBe("/dashboard");
    expect(sectionRoot("/flows/abc123")).toBe("/flows");
  });
});

describe("navigation order", () => {
  const key = (code: string, extra: Partial<KeyboardEventInit> = {}) =>
    new KeyboardEvent("keydown", { key: code.slice(-1), code, ctrlKey: true, ...extra });

  it("Flows sits after Screens: Ctrl+5; Polls takes Chat's old Ctrl+6; Control keeps Ctrl+7", async () => {
    const { NAV_ITEMS, navCommands } = await import("./nav");
    const { registerCommands, runShortcut } = await import("$lib/commands/registry.svelte");
    expect(NAV_ITEMS.map((i) => [i.label, i.shortcut])).toEqual([
      ["Dashboard", "Ctrl+1"],
      ["Templates", "Ctrl+2"],
      ["Sources", "Ctrl+3"],
      ["Screens", "Ctrl+4"],
      ["Flows", "Ctrl+5"],
      ["Polls", "Ctrl+6"],
      ["Control", "Ctrl+7"],
    ]);
    const go = vi.fn();
    const off = registerCommands(navCommands(go));
    try {
      expect(runShortcut(key("Digit5"))).toBe(true);
      expect(go).toHaveBeenLastCalledWith("/flows");
      runShortcut(key("Digit6"));
      expect(go).toHaveBeenLastCalledWith("/polls");
      runShortcut(key("Digit7"));
      expect(go).toHaveBeenLastCalledWith("/control");
      expect(runShortcut(key("Digit8"))).toBe(false);
    } finally {
      off();
    }
  });

  it("palette: Go to Flows (Ctrl+5) and New flow (opens /flows?new)", async () => {
    const { navCommands } = await import("./nav");
    const { currentContext, titleOf } = await import("$lib/commands/registry.svelte");
    const go = vi.fn();
    const cmds = navCommands(go);
    const flows = cmds.find((c) => titleOf(c) === "Go to Flows");
    expect(flows?.id).toBe("nav.flows");
    expect(flows?.defaultKeys).toEqual(["Ctrl+5"]);
    expect(flows?.section).toBe("goto");
    const create = cmds.find((c) => titleOf(c) === "New flow");
    expect(create?.defaultKeys).toBeUndefined();
    expect(create?.section).toBe("create");
    await create?.run?.(currentContext());
    expect(go).toHaveBeenCalledWith("/flows?new");
  });
});

describe("TopBar server status", () => {
  const running: ServerStatus = {
    state: "running",
    port: 7770,
    url: "http://127.0.0.1:7770",
    lanUrl: null,
  };

  async function renderWith(status: ServerStatus) {
    server.status = null;
    api.serverStatus.mockResolvedValue(status);
    render(TopBar);
    await tick();
    await tick();
  }

  const statusRegion = () => screen.getByRole("status");

  it("shows a preview tally dot and the local address in mono while running", async () => {
    await renderWith(running);
    const url = screen.getByText("127.0.0.1:7770");
    expect(url.classList.contains("mono")).toBe(true);
    expect(statusRegion().querySelector(".tally.preview")).toBeTruthy();
    expect(statusRegion().querySelector(".tally.program")).toBeNull();
    expect(screen.getByRole("button", { name: "Copy output URL" })).toBeTruthy();
    expect(api.on).toHaveBeenCalledWith("server-status", expect.any(Function));
  });

  it("shows the LAN address when LAN access is on", async () => {
    await renderWith({ ...running, lanUrl: "http://192.168.1.20:7770" });
    const url = screen.getByText("LAN 192.168.1.20:7770");
    expect(url.classList.contains("mono")).toBe(true);
  });

  it("shows an idle dot while starting", async () => {
    await renderWith({ state: "starting", port: 7770 } as ServerStatus);
    expect(statusRegion().textContent).toContain("Starting…");
    expect(statusRegion().querySelector(".tally.idle")).toBeTruthy();
  });

  it("links a port conflict to Preferences > Server with a warning dot", async () => {
    await renderWith({ state: "error", code: "portInUse", port: 7770, message: "in use" });
    const link = screen.getByRole("link", { name: /Port 7770 is in use/ });
    expect(link.getAttribute("href")).toBe("/settings#server");
    expect(link.querySelector(".tally.warning")).toBeTruthy();
    await fireEvent.click(link);
    expect(api.openPreferences).toHaveBeenCalledWith("server");
  });

  it("explains other bind failures in a tooltip", async () => {
    await renderWith({ state: "error", code: "bindFailed", port: 7770, message: "access denied" });
    expect(statusRegion().textContent).toContain("Server couldn't start");
    expect(statusRegion().querySelector(".tally.warning")).toBeTruthy();
    expect(screen.getByRole("tooltip").textContent).toBe("access denied");
  });

  it("shows nothing when the server is stopped", async () => {
    await renderWith({ state: "stopped" });
    expect(statusRegion().textContent?.trim()).toBe("");
    expect(statusRegion().querySelector(".tally")).toBeNull();
    expect(screen.queryByRole("button", { name: "Copy output URL" })).toBeNull();
  });

  it("copies the base URL (LAN when present)", async () => {
    const writeText = vi.fn(async (_t: string) => {});
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    await renderWith({ ...running, lanUrl: "http://192.168.1.20:7770" });
    await fireEvent.click(screen.getByRole("button", { name: "Copy output URL" }));
    await tick();
    expect(writeText).toHaveBeenCalledWith("http://192.168.1.20:7770");
    expect(toasts.list.map((t) => t.message)).toContain("Copied http://192.168.1.20:7770");
  });

  it("releases the server-status listener on unmount", async () => {
    await renderWith(running);
    cleanup();
    expect(unsubscribe).toHaveBeenCalled();
  });
});

describe("Automation off in the shell", () => {
  it("an amber pill with text in the top bar while it is off; a palette command toggles it", async () => {
    const { currentContext, registry } = await import("$lib/commands/registry.svelte");
    const command = () => registry.list.find((c) => c.id === "shell.automation")!;
    render(AppShell);
    expect(screen.queryByRole("link", { name: "Automation off" })).toBeNull();
    expect(command().title).toBe("Automation off");

    await command().run?.(currentContext());
    await tick();
    expect((api.setUiState.mock.calls.at(-1)![0] as UiState).control.automationOff).toBe(true);
    const pill = screen.getByRole("link", { name: "Automation off" });
    expect(pill.getAttribute("href")).toBe("/control");
    expect(document.querySelector(".top")?.contains(pill)).toBe(true);
    expect(command().title).toBe("Automation on");

    await command().run?.(currentContext());
    await tick();
    expect((api.setUiState.mock.calls.at(-1)![0] as UiState).control.automationOff).toBe(false);
    expect(screen.queryByRole("link", { name: "Automation off" })).toBeNull();
  });
});

describe("Flow log toasts", () => {
  it("a flow's Log toast shows on any page; the listener goes with the shell", async () => {
    let handler: ((p: unknown) => void) | undefined;
    const off = vi.fn();
    api.on.mockImplementation((name: string, h: (p: unknown) => void) => {
      if (name === "flow-log") {
        handler = h;
        return off;
      }
      return unsubscribe;
    });
    render(AppShell);
    handler!({ id: "f1", node: "l1", message: "Doors open", at: "2026-10-04T18:00:00Z" });
    expect(toasts.list.at(-1)).toMatchObject({ kind: "info", message: "Doors open" });
    cleanup();
    expect(off).toHaveBeenCalled();
  });
});

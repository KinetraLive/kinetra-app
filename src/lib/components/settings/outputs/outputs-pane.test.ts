// Preferences › Outputs: watching displays while shown, the map, Add output, status from
// `outputs-changed`, test patterns, and the screen choice without a project.
import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DisplayInfo, OutputConfig, OutputStatus, Prefs } from "$lib/types";

const out = vi.hoisted(() => ({
  listOutputs: vi.fn(async () => [] as OutputStatus[]),
  watchDisplays: vi.fn(async (_on: boolean) => [] as DisplayInfo[]),
  outputTestPattern: vi.fn(async (_id: string, _p: string | null) => {}),
  identifyDisplays: vi.fn(async () => {}),
}));
vi.mock("$lib/api/outputs", () => out);
vi.mock("$lib/api/screens", () => ({
  listScreenDocs: vi.fn(async () => [{ id: "s1", name: "Main program", width: 1920, height: 1080, background: "transparent", layerCount: 2, updatedAt: "" }]),
}));
const api = vi.hoisted(() => ({
  handlers: new Map<string, (p: unknown) => void>(),
  getPrefs: vi.fn(),
  setPrefs: vi.fn(async (p: unknown) => p),
}));
vi.mock("$lib/api", () => ({
  getPrefs: api.getPrefs,
  setPrefs: api.setPrefs,
  toAppError: (e: unknown) => ({ code: "internal", message: String(e) }),
  on: (event: string, h: (p: unknown) => void) => {
    api.handlers.set(event, h);
    return () => api.handlers.delete(event);
  },
}));

import { prefs } from "$lib/stores/prefs.svelte";
import { project } from "$lib/stores/project.svelte";
import OutputsPane from "../panes/OutputsPane.svelte";

const disp = (id: string, x: number, patch: Partial<DisplayInfo> = {}): DisplayInfo => ({
  id,
  name: `Display ${id}`,
  edidKey: "",
  x,
  y: 0,
  width: 1920,
  height: 1080,
  refreshHz: 60,
  primary: false,
  hasMainWindow: false,
  ...patch,
});
const DISPLAYS = [disp("main", 0, { name: "DELL U2720Q", hasMainWindow: true, primary: true }), disp("side", 1920, { name: "ASUS PA248QV" })];
const base: Prefs = { theme: "dark", backup: { enabled: true, intervalMinutes: 15, retention: 10 } };
const fill: OutputConfig = {
  id: "o1",
  kind: "display",
  name: "Output 1",
  enabled: true,
  display: { id: "side", name: "ASUS PA248QV", edidKey: "", width: 1920, height: 1080 },
  screenId: "s1",
  mode: "fill",
  background: { kind: "black" },
  keyOf: null,
  alwaysOnTop: true,
};
const flush = async () => {
  for (let i = 0; i < 4; i++) await tick();
  await new Promise((r) => setTimeout(r, 0));
  await tick();
};

beforeEach(() => {
  vi.clearAllMocks();
  out.watchDisplays.mockResolvedValue(DISPLAYS);
  prefs.set(base);
  project.current = null;
});
afterEach(() => cleanup());

describe("Preferences › Outputs", () => {
  it("on macOS: not available yet, with the docs link; no display watching", async () => {
    const ua = vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_6) AppleWebKit/605.1.15 (KHTML, like Gecko)");
    try {
      render(OutputsPane);
      await flush();
      expect(screen.getByText(/not available on macOS yet/)).toBeTruthy();
      expect(screen.getByRole("link", { name: /What works on macOS/ }).getAttribute("href")).toBe("https://docs.kinetra.live/macos");
      expect(screen.queryByText("No displays found.")).toBeNull();
      expect(screen.queryByRole("button", { name: "Add output" })).toBeNull();
      expect(out.watchDisplays).not.toHaveBeenCalled();
    } finally {
      ua.mockRestore();
    }
  });

  it("watches displays while shown and draws the map", async () => {
    const { unmount } = render(OutputsPane);
    await flush();
    expect(out.watchDisplays).toHaveBeenCalledWith(true);
    const map = screen.getByRole("list", { name: "Displays" });
    expect(within(map).getByText("DELL U2720Q")).toBeTruthy();
    expect(within(map).getByText("Main window")).toBeTruthy();
    expect(within(map).getAllByText("1920×1080 · 60 Hz")).toHaveLength(2);
    // Plugged in: the map follows.
    api.handlers.get("displays-changed")?.([...DISPLAYS, disp("third", 3840, { name: "LG 24MB35" })]);
    await flush();
    expect(within(map).getByText("LG 24MB35")).toBeTruthy();
    unmount();
    expect(out.watchDisplays).toHaveBeenLastCalledWith(false);
    expect(api.handlers.has("displays-changed")).toBe(false);
  });

  it("adds an output on a free display, off; no project: the screen select says so", async () => {
    render(OutputsPane);
    await flush();
    await fireEvent.click(screen.getByRole("button", { name: "Add output" }));
    await flush();
    const saved = api.setPrefs.mock.calls.at(-1)?.[0] as Prefs;
    expect(saved.outputs).toEqual([expect.objectContaining({ name: "Output 1", enabled: false, display: expect.objectContaining({ id: "side" }), screenId: null })]);
    const select = screen.getByRole("combobox", { name: "Screen" }) as HTMLSelectElement;
    expect(select.disabled).toBe(true);
    expect(select.selectedOptions[0].textContent).toBe("Open a project to choose a screen");
  });

  it("shows status, switches, and test patterns pressed while showing", async () => {
    prefs.set({ ...base, outputs: [fill] });
    project.current = { path: "p", name: "P", createdAt: "", settings: {} } as unknown as typeof project.current;
    out.listOutputs.mockResolvedValue([{ id: "o1", state: "showing", displayId: "side", frameLocked: null, pattern: null, message: null }]);
    render(OutputsPane);
    await flush();
    const row = document.querySelector<HTMLElement>('li[data-output="o1"]')!;
    expect(within(row).getByRole("status").textContent).toContain("Showing");
    expect((screen.getByRole("combobox", { name: "Screen" }) as HTMLSelectElement).value).toBe("s1");

    await fireEvent.click(screen.getByRole("button", { name: "Bars" }));
    expect(out.outputTestPattern).toHaveBeenCalledWith("o1", "bars");
    await flush();
    expect(screen.getByRole("button", { name: "Bars" }).getAttribute("aria-pressed")).toBe("true");
    await fireEvent.click(screen.getByRole("button", { name: "Back to screen" }));
    expect(out.outputTestPattern).toHaveBeenLastCalledWith("o1", null);

    // The core reports the display gone: amber text, no tally.
    api.handlers.get("outputs-changed")?.([{ id: "o1", state: "displayMissing", displayId: null, frameLocked: null, pattern: null, message: null }]);
    await flush();
    expect(screen.getByRole("status").textContent).toContain("Display disconnected");

    await fireEvent.click(screen.getByRole("switch", { name: "Show Output 1" }));
    await flush();
    expect((api.setPrefs.mock.calls.at(-1)?.[0] as Prefs).outputs?.[0].enabled).toBe(false);
  });
});

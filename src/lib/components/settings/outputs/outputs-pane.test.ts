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
  ndiInfo: vi.fn(async () => ({ available: true, installed: true, downloadUrl: "https://ndi.link/NDIRedistV6", defaultRate: "50" })),
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
  it("on macOS: display outputs as on Windows, no NDI® (the core says it isn't available)", async () => {
    const ua = vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_6) AppleWebKit/605.1.15 (KHTML, like Gecko)");
    out.ndiInfo.mockResolvedValue({ available: false, installed: false, downloadUrl: "https://ndi.link/NDIRedistV6", defaultRate: "50" });
    try {
      render(OutputsPane);
      await flush();
      expect(out.watchDisplays).toHaveBeenCalledWith(true);
      expect(within(screen.getByRole("list", { name: "Displays" })).getByText("DELL U2720Q")).toBeTruthy();
      expect(screen.getByRole("button", { name: "Add output" })).toBeTruthy();
      expect(screen.queryByRole("button", { name: /NDI/ })).toBeNull();
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

  it("adds an NDI® output with the region's rate; its row, status and the trademark line", async () => {
    project.current = { path: "p", name: "P", createdAt: "", settings: {} } as unknown as typeof project.current;
    out.ndiInfo.mockResolvedValue({ available: true, installed: false, downloadUrl: "https://ndi.link/NDIRedistV6", defaultRate: "59.94" });
    render(OutputsPane);
    await flush();
    expect(screen.getByText("NDI® is a registered trademark of Vizrt NDI AB.")).toBeTruthy();
    expect(screen.getByText(/need the free NDI® Runtime/)).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Add NDI® output" }));
    await flush();
    const saved = (api.setPrefs.mock.calls.at(-1)?.[0] as Prefs).outputs![0];
    expect(saved).toMatchObject({ kind: "ndi", name: "Kinetra Main program", enabled: false, screenId: "s1", ndi: { rate: "59.94", alpha: true } });
    prefs.set({ ...base, uiMode: "simple", outputs: [saved] });
    api.handlers.get("outputs-changed")?.([
      { id: saved.id, state: "showing", displayId: null, frameLocked: null, pattern: null, message: null, ndi: { receivers: 1, program: true, preview: false, dropping: false } },
    ]);
    await flush();
    const row = document.querySelector<HTMLElement>(`li[data-output="${saved.id}"]`)!;
    expect(within(row).getByRole("status").textContent).toContain("On program");
    expect(within(row).getByRole("link", { name: /About NDI®/ }).getAttribute("href")).toBe("https://ndi.video");
    expect((within(row).getByRole("combobox", { name: "Frame rate" }) as HTMLSelectElement).value).toBe("59.94");
    // Simple mode hides the Advanced settings and says so.
    expect(within(row).queryByRole("textbox", { name: "NDI® groups" })).toBeNull();
    expect(screen.getByText(/More NDI® settings/)).toBeTruthy();
    // The runtime missing: amber, with the download link.
    api.handlers.get("outputs-changed")?.([{ id: saved.id, state: "ndiMissing", displayId: null, frameLocked: null, pattern: null, message: null, ndi: { receivers: 0, program: false, preview: false, dropping: false } }]);
    await flush();
    expect(within(row).getByRole("status").textContent).toContain("NDI® Runtime not installed");
    expect(within(row).getByRole("link", { name: /Download/ }).getAttribute("href")).toBe("https://ndi.link/NDIRedistV6");
    // Back from installing it: the window's focus asks again (which starts waiting outputs), once installed no more.
    const asked = out.ndiInfo.mock.calls.length;
    out.ndiInfo.mockResolvedValue({ available: true, installed: true, downloadUrl: "https://ndi.link/NDIRedistV6", defaultRate: "59.94" });
    window.dispatchEvent(new Event("focus"));
    await flush();
    expect(out.ndiInfo.mock.calls.length).toBe(asked + 1);
    expect(screen.queryByText(/need the free NDI® Runtime/)).toBeNull();
    window.dispatchEvent(new Event("focus"));
    await flush();
    expect(out.ndiInfo.mock.calls.length).toBe(asked + 1);
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

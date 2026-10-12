import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AutoAdvance, ServerStatus, SourceSummary } from "$lib/types";

const api = vi.hoisted(() => ({
  on: vi.fn(() => () => {}),
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  serverStatus: vi.fn(),
  listScreens: vi.fn(),
  setAutoAdvance: vi.fn(),
  stepSource: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) => ({ code: "internal", message: e instanceof Error ? e.message : String(e) }),
}));
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/screens", () => api);
vi.mock("$lib/api/sources", () => api);
vi.mock("$lib/api/templates", () => api);
vi.mock("$lib/api/automation", () => api);

import SlideshowPanel from "./SlideshowPanel.svelte";
import { ControlCtl, DEFAULT_AUTO } from "$lib/components/control/control.svelte";
import { server } from "$lib/stores/server.svelte";
import { toasts } from "$lib/stores/toasts.svelte";

const running = { state: "running", port: 7770, url: "http://127.0.0.1:7770", lanUrl: null } satisfies ServerStatus;

function sponsors(patch: Partial<SourceSummary> = {}): SourceSummary {
  return {
    id: "sp",
    name: "Sponsors",
    kind: "text",
    rowCount: 3,
    index: 1,
    current: { logo: "/assets/acme.png", name: "Acme", tagline: "Rockets" },
    columns: [
      { key: "logo", label: "Logo", kind: "image" },
      { key: "name", label: "Name" },
      { key: "tagline", label: "Tagline" },
    ],
    usedBy: ["s1"],
    updatedAt: "2026-01-01T00:00:00Z",
    auto: null,
    nextAt: null,
    nowId: null,
    nextId: null,
    ...patch,
  };
}

const flush = async () => {
  for (let i = 0; i < 3; i++) {
    await Promise.resolve();
    await tick();
  }
};

function mount(s: SourceSummary) {
  const ctl = new ControlCtl();
  ctl.sources = [s];
  render(SlideshowPanel, { props: { ctl, sourceId: s.id } });
  return ctl;
}

const lastAuto = () => api.setAutoAdvance.mock.calls.at(-1) as [string, AutoAdvance | null];

beforeEach(() => {
  vi.clearAllMocks();
  api.serverStatus.mockResolvedValue(running);
  server.status = running;
  api.setAutoAdvance.mockImplementation(async (_id: string, auto: AutoAdvance | null) => sponsors({ auto }));
  api.stepSource.mockImplementation(async () => sponsors());
});

afterEach(() => {
  cleanup();
  toasts.clear();
});

describe("slideshow panel", () => {
  it("shows the current slide image from the output server and the position", async () => {
    mount(sponsors());
    await flush();
    const img = screen.getByRole("img", { name: "Acme · Rockets" }) as HTMLImageElement;
    expect(img.src).toBe("http://127.0.0.1:7770/assets/acme.png");
    expect(img.getAttribute("loading")).toBe("lazy");
    expect(screen.getByText("2 / 3")).toBeTruthy();
    expect(screen.getByRole("status").textContent?.trim()).toBe("Stopped");
  });

  it("a text slide shows its text", async () => {
    mount(sponsors({ columns: [{ key: "name", label: "Name" }], current: { name: "Acme" } }));
    await flush();
    expect(screen.queryByRole("img")).toBeNull();
    expect(screen.getByText("Acme")).toBeTruthy();
  });

  it("Play turns auto-advance on; Pause and Play again toggle `paused`", async () => {
    const ctl = mount(sponsors());
    await fireEvent.click(screen.getByRole("button", { name: "Play" }));
    await flush();
    expect(lastAuto()).toEqual(["sp", DEFAULT_AUTO]);

    ctl.sources = [sponsors({ auto: { ...DEFAULT_AUTO }, nextAt: Date.now() + 4000 })];
    await flush();
    expect(screen.getByRole("status").textContent?.trim()).toBe("Playing");
    expect(document.querySelector("[data-progress]")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    await flush();
    expect(lastAuto()).toEqual(["sp", { ...DEFAULT_AUTO, paused: true }]);

    ctl.sources = [sponsors({ auto: { ...DEFAULT_AUTO, paused: true } })];
    await flush();
    expect(document.querySelector("[data-progress]")).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Play" }));
    await flush();
    expect(lastAuto()).toEqual(["sp", { ...DEFAULT_AUTO, paused: false }]);
  });

  it("Previous / Next step the source; with Loop, Next on the last slide wraps to the first", async () => {
    const ctl = mount(sponsors({ auto: { ...DEFAULT_AUTO, loop: false } }));
    await fireEvent.click(screen.getByRole("button", { name: "Next slide" }));
    await flush();
    expect(api.stepSource).toHaveBeenLastCalledWith("sp", { to: "by", delta: 1 });
    await fireEvent.click(screen.getByRole("button", { name: "Previous slide" }));
    await flush();
    expect(api.stepSource).toHaveBeenLastCalledWith("sp", { to: "by", delta: -1 });

    ctl.sources = [sponsors({ index: 2, auto: { ...DEFAULT_AUTO, loop: false } })];
    await flush();
    expect((screen.getByRole("button", { name: "Next slide" }) as HTMLButtonElement).disabled).toBe(true);

    ctl.sources = [sponsors({ index: 2, auto: { ...DEFAULT_AUTO, loop: true } })];
    await flush();
    await fireEvent.click(screen.getByRole("button", { name: "Next slide" }));
    await flush();
    expect(api.stepSource).toHaveBeenLastCalledWith("sp", { to: "index", index: 0 });
  });

  it("interval, shuffle, per-slide durations and loop go to setAutoAdvance while on", async () => {
    mount(sponsors({ auto: { ...DEFAULT_AUTO } }));
    const every = screen.getByLabelText("Every") as HTMLInputElement;
    every.value = "8";
    await fireEvent.change(every);
    await flush();
    expect(lastAuto()).toEqual(["sp", { ...DEFAULT_AUTO, everyMs: 8000 }]);
    await fireEvent.click(screen.getByRole("switch", { name: "Shuffle" }));
    await flush();
    expect(lastAuto()[1]?.shuffle).toBe(true);
    await fireEvent.click(screen.getByRole("switch", { name: "Per-slide durations" }));
    await flush();
    expect(lastAuto()[1]?.itemDurations).toBe(true);
    await fireEvent.click(screen.getByRole("switch", { name: "Loop" }));
    await flush();
    expect(lastAuto()[1]?.loop).toBe(false);
    await fireEvent.click(screen.getByRole("radio", { name: "Play out & in" }));
    await flush();
    expect(lastAuto()[1]?.transition).toBe("retake");
  });

  it("settings changed while off are remembered for the next Play", async () => {
    mount(sponsors());
    await fireEvent.click(screen.getByRole("switch", { name: "Shuffle" }));
    await flush();
    expect(api.setAutoAdvance).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole("button", { name: "Play" }));
    await flush();
    expect(lastAuto()).toEqual(["sp", { ...DEFAULT_AUTO, shuffle: true }]);
  });
});

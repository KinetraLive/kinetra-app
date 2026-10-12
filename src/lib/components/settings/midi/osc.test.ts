// Preferences > Server > OSC: the listener status line (model and the block following
// `osc_status` / `osc-status`) and the address list matching the core's scheme.
import { cleanup, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { OscStatus } from "$lib/types";

const osc = vi.hoisted(() => ({
  handler: null as ((s: unknown) => void) | null,
  unsubscribed: 0,
  status: vi.fn(),
}));
vi.mock("$lib/api/midi", () => ({
  oscStatus: osc.status,
  onOscStatus: (h: (s: unknown) => void) => {
    osc.handler = h;
    return () => {
      osc.handler = null;
      osc.unsubscribed++;
    };
  },
}));
vi.mock("$lib/api/flows", () => ({ listFlows: async () => [] }));
vi.mock("$lib/api/live", () => ({ listScreens: async () => [] }));
vi.mock("$lib/api/sources", () => ({ listSources: async () => [] }));
vi.mock("$lib/stores/project.svelte", () => ({ project: { current: null } }));
vi.mock("../save", () => ({ saveProjectSettings: vi.fn(async () => true) }));

import Osc from "./Osc.svelte";
import { oscAddresses, oscStatusLine } from "./osc";

const settle = async () => {
  for (let i = 0; i < 4; i++) await tick();
};
const statusText = () => screen.queryByRole("status")?.textContent?.replace(/\s+/g, " ").trim() ?? null;

describe("OSC status line model", () => {
  it("maps every listener state", () => {
    expect(oscStatusLine(null)).toBeNull();
    expect(oscStatusLine({ state: "off" })).toBeNull();
    expect(oscStatusLine({ state: "listening", port: 9000, lan: true })).toEqual({ kind: "listening", port: 9000, lan: true });
    expect(oscStatusLine({ state: "error", code: "portInUse", port: 9000, message: "x" })).toEqual({ kind: "portInUse", port: 9000 });
    expect(oscStatusLine({ state: "error", code: "bindFailed", port: 9000, message: "Access denied" })).toEqual({ kind: "failed", message: "Access denied" });
  });
});

describe("OSC block status", () => {
  beforeEach(() => {
    osc.handler = null;
    osc.unsubscribed = 0;
    osc.status.mockReset();
  });
  afterEach(() => cleanup());

  async function mount(initial: OscStatus) {
    osc.status.mockResolvedValue(initial);
    const r = render(Osc);
    await settle();
    return r;
  }

  it("shows nothing when off", async () => {
    await mount({ state: "off" });
    expect(statusText()).toBeNull();
  });

  it("shows the port when listening, with LAN access noted", async () => {
    await mount({ state: "listening", port: 9000, lan: false });
    expect(statusText()).toBe("Listening on 9000");
    expect(screen.getByText("9000").classList.contains("mono")).toBe(true);
    osc.handler?.({ state: "listening", port: 9001, lan: true });
    await settle();
    expect(statusText()).toBe("Listening on 9001 · other devices allowed");
  });

  it("warns when the port is in use and shows other bind failures", async () => {
    await mount({ state: "error", code: "portInUse", port: 9000, message: "in use" });
    expect(statusText()).toBe("Port 9000 is in use — choose another port.");
    expect(screen.getByText(/is in use/).classList.contains("warn")).toBe(true);
    osc.handler?.({ state: "error", code: "bindFailed", port: 9000, message: "Couldn't open the OSC port." });
    await settle();
    expect(statusText()).toBe("Couldn't open the OSC port.");
    osc.handler?.({ state: "off" });
    await settle();
    expect(statusText()).toBeNull();
  });

  it("an event before the first answer wins, and unmounting unsubscribes", async () => {
    let answer: (s: OscStatus) => void = () => {};
    osc.status.mockReturnValue(new Promise<OscStatus>((r) => (answer = r)));
    const { unmount } = render(Osc);
    await settle();
    osc.handler?.({ state: "listening", port: 9100, lan: false });
    answer({ state: "off" });
    await settle();
    expect(statusText()).toBe("Listening on 9100");
    unmount();
    expect(osc.unsubscribed).toBe(1);
    expect(osc.handler).toBeNull();
  });
});

describe("OSC addresses", () => {
  it("match the core's scheme exactly, with ids filled in", () => {
    const list = oscAddresses({ screenId: "s1", layerId: "l1", flowId: "f1", sourceId: "src1" });
    expect(list.map((a) => [a.address, a.arg ?? null])).toEqual([
      ["/kinetra/layer/s1/l1/take", null],
      ["/kinetra/layer/s1/l1/clear", null],
      ["/kinetra/layer/s1/l1/toggle", null],
      ["/kinetra/layer/s1/l1/update", null],
      ["/kinetra/layer/s1/l1/opacity", "0..1"],
      ["/kinetra/layer/s1/l1/var/<key>", "0..1"],
      ["/kinetra/layer/s1/l1/play", null],
      ["/kinetra/layer/s1/l1/pause", null],
      ["/kinetra/layer/s1/l1/stop", null],
      ["/kinetra/layer/s1/l1/seek", "seconds"],
      ["/kinetra/flow/f1/go", null],
      ["/kinetra/source/src1/next", null],
      ["/kinetra/source/src1/prev", null],
      ["/kinetra/screen/s1/clear", null],
      ["/kinetra/clear-all", null],
      ["/kinetra/panic", null],
    ]);
  });

  it("uses placeholders where nothing is picked", () => {
    const list = oscAddresses({ screenId: null, layerId: null, flowId: null, sourceId: null, varKey: "size" });
    expect(list.map((a) => a.address)).toContain("/kinetra/layer/<screenId>/<layerId>/update");
    expect(list.map((a) => a.address)).toContain("/kinetra/layer/<screenId>/<layerId>/var/size");
    expect(list.map((a) => a.address)).toContain("/kinetra/screen/<screenId>/clear");
  });
});

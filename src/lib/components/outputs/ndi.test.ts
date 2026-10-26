// NDI® outputs, the pure part: status lines (on air only with a receiver; no receiver count
// shown), warnings, the Exit rule, names, adding one and the per-screen switch.
import { describe, expect, it } from "vitest";
import type { NdiLive, OutputConfig, OutputStatus } from "$lib/types";
import {
  canAddNdi,
  fillOptions,
  isOnAir,
  menuLabel,
  ndiName,
  ndiOutputOf,
  newNdiOutput,
  showing,
  statusLine,
  warningText,
  withMode,
  withNdi,
  withScreenNdi,
} from "./model";

const ndi = (id: string, patch: Partial<OutputConfig> = {}): OutputConfig => ({
  id,
  kind: "ndi",
  name: `Kinetra ${id}`,
  enabled: true,
  display: { id: "", name: "", edidKey: "", width: 0, height: 0 },
  screenId: "main",
  mode: "fill",
  background: { kind: "black" },
  keyOf: null,
  alwaysOnTop: false,
  ndi: { rate: "50", alpha: true, premultiplied: false, groups: null, bus: "program" },
  ...patch,
});
const display = (id: string): OutputConfig => ({
  ...ndi(id),
  kind: "display",
  name: `Output ${id}`,
  display: { id: "d1", name: "DELL", edidKey: "", width: 1920, height: 1080 },
  ndi: undefined,
});
const st = (id: string, state: OutputStatus["state"], live?: Partial<NdiLive>): OutputStatus => ({
  id,
  state,
  displayId: null,
  frameLocked: null,
  pattern: null,
  message: null,
  ndi: { receivers: 0, program: false, preview: false, dropping: false, ...live },
});

describe("NDI outputs", () => {
  it("are on air only while a receiver has them", () => {
    expect(isOnAir(st("n", "showing"))).toBe(false);
    expect(isOnAir(st("n", "showing", { receivers: 1 }))).toBe(true);
    expect(isOnAir(st("n", "ndiMissing", { receivers: 1 }))).toBe(false);
    // Display outputs: while showing.
    expect(isOnAir({ ...st("o", "showing"), ndi: undefined })).toBe(true);
    const configs = [ndi("a"), ndi("b"), display("c")];
    const status = [st("a", "showing"), st("b", "showing", { receivers: 2 }), { ...st("c", "showing"), ndi: undefined }];
    expect(showing(configs, status).map((o) => o.id)).toEqual(["b", "c"]);
  });

  it("status lines: tally first, no receiver count, problems in amber", () => {
    const o = ndi("n");
    expect(statusLine(o, st("n", "showing"))).toEqual({ tone: "muted", text: "Ready · no receiver" });
    expect(statusLine(o, st("n", "showing", { receivers: 2 }))).toEqual({ tone: "program", text: "Sending" });
    expect(statusLine(o, st("n", "showing", { receivers: 1, program: true }))).toEqual({ tone: "program", text: "On program" });
    expect(statusLine(o, st("n", "showing", { receivers: 1, preview: true }))).toEqual({ tone: "preview", text: "On preview" });
    expect(statusLine(o, st("n", "showing", { receivers: 1, dropping: true }))).toEqual({ tone: "warning", text: "Dropping frames" });
    expect(statusLine(o, st("n", "ndiMissing"))).toEqual({ tone: "warning", text: "NDI® Runtime not installed" });
    expect(statusLine(o, { ...st("n", "failed"), message: "NDI output isn't available on this computer yet" }).text).toBe(
      "NDI output isn't available on this computer yet",
    );
    expect(statusLine(o, st("n", "noProject")).text).toBe("Waiting for a project");
    expect(statusLine({ ...o, enabled: false }, st("n", "off")).text).toBe("Off");
  });

  it("warnings and menu labels", () => {
    const o = ndi("n", { name: "Kinetra Main" });
    expect(warningText(o, st("n", "ndiMissing"))).toBe("Kinetra Main: NDI® Runtime not installed");
    expect(warningText(o, st("n", "showing", { dropping: true }))).toBe("Kinetra Main: dropping frames");
    expect(warningText(o, st("n", "showing", { receivers: 1 }))).toBeNull();
    expect(menuLabel(o)).toBe("Kinetra Main — NDI®");
  });

  it("can't be keyed: key outputs only pair with display fills", () => {
    const configs = [ndi("n"), display("o")];
    expect(fillOptions(configs, "k").map((f) => f.value)).toEqual(["o"]);
    expect(withMode([ndi("n"), { ...display("k") }], "k", "key")).toBeNull();
  });

  it("names: Kinetra <screen>, made unique", () => {
    expect(ndiName([], "Main")).toBe("Kinetra Main");
    expect(ndiName([ndi("x", { name: "kinetra main" })], "Main")).toBe("Kinetra Main 2");
    expect(ndiName([], null)).toBe("Kinetra");
    expect(ndiName([], "x".repeat(60))).toHaveLength(40);
  });

  it("a new NDI output: the region's rate, transparency, straight alpha, Program, off", () => {
    const o = newNdiOutput([], { id: "main", name: "Main" }, "59.94")!;
    expect(o).toMatchObject({
      kind: "ndi",
      name: "Kinetra Main",
      enabled: false,
      screenId: "main",
      alwaysOnTop: false,
      ndi: { rate: "59.94", alpha: true, premultiplied: false, groups: null, bus: "program" },
    });
    expect(o.id).toMatch(/^[0-9a-z]{10}$/);
    // At most 4 NDI outputs and 8 outputs in all.
    const four = ["a", "b", "c", "d"].map((i) => ndi(i));
    expect(canAddNdi(four)).toBe(false);
    expect(newNdiOutput(four, null, "50")).toBeNull();
    const eight = [...four.slice(0, 1), ...["1", "2", "3", "4", "5", "6", "7"].map(display)];
    expect(canAddNdi(eight)).toBe(false);
  });

  it("Send / Stop sending a screen over NDI®", () => {
    const screen = { id: "main", name: "Main" };
    // None yet: one is made, on, with the given rate.
    const made = withScreenNdi([display("o")], screen, true, "50")!;
    expect(made).toHaveLength(2);
    expect(made[1]).toMatchObject({ kind: "ndi", enabled: true, screenId: "main", ndi: { rate: "50" } });
    // Existing: switched, nothing added.
    const off = withScreenNdi(made, screen, false, "50")!;
    expect(off).toHaveLength(2);
    expect(ndiOutputOf(off, "main")?.enabled).toBe(false);
    expect(withScreenNdi([], screen, false, "50")).toEqual([]);
    // No room: null.
    const four = ["a", "b", "c", "d"].map((i) => ndi(i, { screenId: "other" }));
    expect(withScreenNdi(four, screen, true, "50")).toBeNull();
  });

  it("settings edits keep the rest", () => {
    const o = ndi("n");
    expect(withNdi(o, { rate: "25" }).ndi).toEqual({ ...o.ndi, rate: "25" });
    expect(withNdi({ ...o, ndi: undefined }, { alpha: false }).ndi).toMatchObject({ alpha: false, rate: "50" });
  });
});

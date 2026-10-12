// Display outputs, the pure part: status lines, warnings, the display map, choices and edits.
import { describe, expect, it } from "vitest";
import { OUTPUTS_MAX, type DisplayInfo, type OutputConfig, type OutputStatus } from "$lib/types";
import {
  displayOptions,
  fillOptions,
  frameLockText,
  mapLayout,
  menuLabel,
  newOutput,
  nextName,
  removeOutput,
  screenOf,
  showing,
  showingText,
  statusLine,
  warningText,
  withBackground,
  withMode,
} from "./model";

const disp = (id: string, x: number, patch: Partial<DisplayInfo> = {}): DisplayInfo => ({
  id,
  name: `Display ${id}`,
  edidKey: `edid-${id}`,
  x,
  y: 0,
  width: 1920,
  height: 1080,
  refreshHz: 60,
  primary: false,
  hasMainWindow: false,
  ...patch,
});
const out = (id: string, patch: Partial<OutputConfig> = {}): OutputConfig => ({
  id,
  kind: "display",
  name: `Output ${id}`,
  enabled: true,
  display: { id: "d1", name: "DELL U2720Q", edidKey: "", width: 1920, height: 1080 },
  screenId: "s1",
  mode: "fill",
  background: { kind: "black" },
  keyOf: null,
  alwaysOnTop: true,
  ...patch,
});
const st = (id: string, patch: Partial<OutputStatus> = {}): OutputStatus => ({
  id,
  state: "showing",
  displayId: "d1",
  frameLocked: null,
  pattern: null,
  message: null,
  ...patch,
});

describe("labels and status", () => {
  it("reads each state as one line with its tone", () => {
    const o = out("a");
    expect(statusLine(o, st("a"))).toEqual({ tone: "program", text: "Showing" });
    expect(statusLine(o, st("a", { pattern: "bars" }))).toEqual({ tone: "program", text: "Test pattern" });
    expect(statusLine(o, st("a", { state: "displayMissing" }))).toEqual({ tone: "warning", text: "Display disconnected" });
    expect(statusLine(o, st("a", { state: "screenMissing" }))).toEqual({ tone: "warning", text: "Screen not found" });
    expect(statusLine(o, st("a", { state: "failed", message: "Display used by Output 2" })).text).toBe("Display used by Output 2");
    expect(statusLine(o, st("a", { state: "noProject" })).tone).toBe("muted");
    expect(statusLine(out("a", { enabled: false }), undefined)).toEqual({ tone: "muted", text: "Off" });
  });

  it("labels key pairs and menu items", () => {
    expect(frameLockText(st("a", { frameLocked: true }))).toBe("Frame-locked");
    expect(frameLockText(st("a", { frameLocked: false }))).toBe("Not frame-locked: displays differ");
    expect(frameLockText(st("a"))).toBeNull();
    expect(menuLabel(out("1", { name: "Output 1" }))).toBe("Output 1 — DELL U2720Q");
    expect(showingText(1)).toBe("1 output is showing.");
    expect(showingText(2)).toBe("2 outputs are showing.");
  });

  it("warns about enabled outputs that can't show", () => {
    const o = out("a", { name: "Output 2" });
    expect(warningText(o, st("a", { state: "displayMissing" }))).toBe("Output 2: display disconnected");
    expect(warningText(o, st("a", { state: "screenMissing" }))).toBe("Output 2: screen not found");
    expect(warningText(o, st("a"))).toBeNull();
    expect(warningText({ ...o, enabled: false }, st("a", { state: "displayMissing" }))).toBeNull();
  });

  it("lists the showing outputs and a key output's screen (its fill's)", () => {
    const fill = out("f", { screenId: "s9" });
    const key = out("k", { mode: "key", keyOf: "f", screenId: null });
    expect(screenOf([fill, key], key)).toBe("s9");
    expect(showing([fill, key], [st("f"), st("k", { state: "off" })]).map((o) => o.id)).toEqual(["f"]);
  });
});

describe("display map", () => {
  it("places displays as percentages of the desktop", () => {
    const l = mapLayout([disp("a", 0, { width: 2560, height: 1440 }), disp("b", 2560)]);
    expect(l.aspect).toBeCloseTo(4480 / 1440);
    expect(l.boxes[1]).toMatchObject({ left: 57.14, top: 0, width: 42.86, height: 75 });
  });

  it("handles negative positions and no displays", () => {
    const l = mapLayout([disp("a", -1920), disp("b", 0)]);
    expect(l.boxes.map((b) => b.left)).toEqual([0, 50]);
    expect(mapLayout([]).boxes).toEqual([]);
  });
});

describe("choices", () => {
  it("offers connected displays and the remembered one when it is gone", () => {
    const o = out("a", { display: { id: "gone", name: "LG 24MB35", edidKey: "", width: 1920, height: 1080 } });
    expect(displayOptions(o, undefined, [disp("d1", 0)]).map((x) => x.label)).toEqual(["Display d1 · 1920×1080", "LG 24MB35 (disconnected)"]);
    // A fallback match (moved to another port) selects the display it is on now.
    expect(displayOptions(o, st("a", { displayId: "d1" }), [disp("d1", 0)])).toHaveLength(1);
  });

  it("lets a key output key for other fill outputs only", () => {
    const list = [out("a"), out("b", { mode: "key", keyOf: "a" }), out("c")];
    expect(fillOptions(list, "c").map((x) => x.value)).toEqual(["a"]);
    expect(fillOptions(list, "b").map((x) => x.value)).toEqual(["a", "c"]);
  });
});

describe("edits", () => {
  it("adds an output on the first free display, off, named Output N", () => {
    const displays = [disp("main", 0, { hasMainWindow: true }), disp("d1", 1920), disp("d2", 3840)];
    const o = newOutput([out("x", { name: "Output 1" })], displays, "s1");
    expect(o).toMatchObject({ name: "Output 2", enabled: false, mode: "fill", display: { id: "d2" }, screenId: "s1", keyOf: null });
    expect(o?.id).toMatch(/^[0-9a-z]{10}$/);
    // Every display used: the first one.
    expect(newOutput([out("x", { display: { ...out("x").display, id: "only" } })], [disp("only", 0)], null)?.display.id).toBe("only");
    expect(newOutput([], [], null)).toBeNull();
    expect(newOutput(Array.from({ length: OUTPUTS_MAX }, (_, i) => out(String(i))), displays, null)).toBeNull();
  });

  it("names outputs by the lowest free number, ignoring case", () => {
    expect(nextName([out("a", { name: "output 1" }), out("b", { name: "Output 3" })])).toBe("Output 2");
  });

  it("switches fill and key", () => {
    const list = [out("a"), out("b")];
    expect(withMode(list, "b", "key")?.[1]).toMatchObject({ mode: "key", keyOf: "a" });
    expect(withMode([out("a")], "a", "key")).toBeNull();
    expect(withMode([out("a"), out("b", { mode: "key", keyOf: "a" })], "b", "fill")?.[1]).toMatchObject({ mode: "fill", keyOf: null });
  });

  it("deleting a fill turns its key outputs into fill outputs", () => {
    const next = removeOutput([out("a"), out("b", { mode: "key", keyOf: "a" })], "a");
    expect(next).toEqual([out("b", { mode: "fill", keyOf: null })]);
  });

  it("keeps the last custom color", () => {
    expect(withBackground(out("a", { background: { kind: "color", color: "#123456" } }), "color")).toEqual({ kind: "color", color: "#123456" });
    expect(withBackground(out("a"), "color")).toEqual({ kind: "color", color: "#808080" });
    expect(withBackground(out("a"), "green")).toEqual({ kind: "green" });
  });
});

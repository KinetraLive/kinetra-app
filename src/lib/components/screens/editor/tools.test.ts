// Layout tools maths: smart guides and equal spacing, align / distribute, rulers and grid, the
// editor's commands, shortcut matching and the command registry.
import { afterEach, describe, expect, it, vi } from "vitest";
import { matchesShortcut } from "$lib/commands/keys";
import { currentContext, registerCommands, registry, runShortcut } from "$lib/commands/registry.svelte";
import { align, distribute } from "./align";
import { editorCommands, nextCrop, type EditorActions } from "./commands";
import { snapMove, snapTargets } from "./geometry";
import { DEFAULT_SCREEN_EDITOR } from "./limits";
import { rulerStep, rulerTicks, visibleGrid } from "./ruler";
import { smartContext, smartGuides, snapSpacing } from "./smart";
import type { ScreenEditorView } from "$lib/types";

const HD = { w: 1920, h: 1080 };
const r = (x: number, y: number, w: number, h: number) => ({ x, y, w, h });

describe("smart guides", () => {
  // A row: A [100, 200] and B [300, 400], gap 100.
  const A = r(100, 100, 100, 100);
  const B = r(300, 100, 100, 100);
  const ctx = smartContext(HD, [B, A]);

  it("snaps to equal spacing: the gap to the neighbour equals a gap in the row", () => {
    const raw = r(497, 100, 100, 100); // gap to B: 97
    const s = snapSpacing(raw, snapMove(raw, snapTargets(HD, [A, B]), 6), ctx, 6);
    expect(s.rect.x).toBe(500);
    expect(s.guides.some((g) => g.axis === "x")).toBe(false);
    // Before the first box too: A.x - 100 - w.
    const left = r(-87, 100, 90, 100);
    expect(snapSpacing(left, { rect: left, guides: [] }, ctx, 6).rect.x).toBe(-90);
  });

  it("snaps to the middle between two neighbours (equal gaps on both sides)", () => {
    const wide = smartContext(HD, [r(100, 100, 100, 100), r(500, 100, 100, 100)]);
    const raw = r(297, 120, 100, 50);
    expect(snapSpacing(raw, { rect: raw, guides: [] }, wide, 6).rect.x).toBe(300);
    const g = smartGuides(r(300, 120, 100, 50), wide);
    expect(g.spacing.map((s) => [s.a, s.b])).toEqual([
      [200, 300],
      [400, 500],
    ]);
  });

  it("a line snap nearer than the spacing target wins; nothing beyond the threshold or with Alt", () => {
    const raw = r(503, 100, 100, 100);
    const lined = { rect: r(502, 100, 100, 100), guides: [{ axis: "x" as const, pos: 502 }] };
    expect(snapSpacing(raw, lined, ctx, 6)).toEqual(lined);
    const far = r(520, 100, 100, 100);
    expect(snapSpacing(far, { rect: far, guides: [] }, ctx, 6).rect.x).toBe(520);
    expect(snapSpacing(raw, { rect: raw, guides: [] }, ctx, 0).rect.x).toBe(503);
  });

  it("measures the distance to the nearest neighbour or the screen edge on each side", () => {
    const g = smartGuides(r(500, 100, 100, 100), ctx);
    const by = (axis: "x" | "y") => g.measures.filter((m) => m.axis === axis).map((m) => [m.a, m.b, m.at]);
    // Row: B on the left (gap 100, at the middle of the overlap, y 150), the screen edge on the right.
    expect(by("x")).toEqual([
      [400, 500, 150],
      [600, 1920, 150],
    ]);
    // Column: nothing above or below, so the screen edges (at the box's centre x).
    expect(by("y")).toEqual([
      [0, 100, 550],
      [200, 1080, 550],
    ]);
    // Equal-spacing markers: our gap to B and the A–B gap.
    expect(g.spacing.map((s) => [s.axis, s.a, s.b])).toEqual([
      ["x", 400, 500],
      ["x", 200, 300],
    ]);
  });

  it("boxes outside the row don't count", () => {
    const g = smartGuides(r(500, 600, 100, 100), ctx);
    expect(g.measures.find((m) => m.axis === "x" && m.b === 500)).toEqual({ axis: "x", a: 0, b: 500, at: 650 });
    expect(g.spacing).toEqual([]);
  });

  it("stays cheap per frame with 64 layers (precomputed sorted lists, linear filtering)", () => {
    const others = Array.from({ length: 63 }, (_, i) => r((i % 8) * 230, Math.floor(i / 8) * 130, 200, 100));
    const big = smartContext(HD, others);
    const targets = snapTargets(HD, others);
    const t0 = performance.now();
    for (let i = 0; i < 200; i++) {
      const raw = r(400 + (i % 50), 300, 180, 90);
      const s = snapSpacing(raw, snapMove(raw, targets, 6), big, 6);
      smartGuides(s.rect, big);
    }
    expect((performance.now() - t0) / 200).toBeLessThan(1);
  });
});

describe("align and distribute", () => {
  const rects = [r(100, 100, 200, 100), r(400, 300, 100, 50), r(250, 600, 300, 200)];

  it("aligns two or more to the selection's bounds", () => {
    // Bounds: x 100..550, y 100..800.
    expect(align(rects, "left", HD).map((b) => b.x)).toEqual([100, 100, 100]);
    expect(align(rects, "right", HD).map((b) => b.x)).toEqual([350, 450, 250]);
    expect(align(rects, "center", HD).map((b) => b.x)).toEqual([225, 275, 175]);
    expect(align(rects, "top", HD).map((b) => b.y)).toEqual([100, 100, 100]);
    expect(align(rects, "bottom", HD).map((b) => b.y)).toEqual([700, 750, 600]);
    expect(align(rects, "middle", HD).map((b) => b.y)).toEqual([400, 425, 350]);
    // Only the aligned axis changes.
    expect(align(rects, "left", HD).map((b) => [b.y, b.w, b.h])).toEqual(rects.map((b) => [b.y, b.w, b.h]));
  });

  it("aligns one layer to the screen", () => {
    const one = [r(100, 100, 400, 200)];
    expect(align(one, "center", HD)).toEqual([r(760, 100, 400, 200)]);
    expect(align(one, "right", HD)).toEqual([r(1520, 100, 400, 200)]);
    expect(align(one, "middle", HD)).toEqual([r(100, 440, 400, 200)]);
    expect(align(one, "bottom", HD)).toEqual([r(100, 880, 400, 200)]);
    expect(align([], "left", HD)).toEqual([]);
  });

  it("distributes three or more with equal gaps, keeping the outer ones", () => {
    const row = [r(0, 0, 100, 10), r(700, 0, 100, 10), r(150, 0, 200, 10)];
    // Span 0..800, sizes 400: two gaps of 200. Sorted by x: 0, 150, 700 → 0, 300, 700.
    expect(distribute(row, "x").map((b) => b.x)).toEqual([0, 700, 300]);
    const col = [r(0, 0, 10, 100), r(0, 333, 10, 100), r(0, 1000, 10, 100), r(0, 450, 10, 50)];
    // Span 0..1100, sizes 350: three gaps of 250: 0, 350, 700 (after the 50-high one), 1000.
    expect(distribute(col, "y").map((b) => b.y)).toEqual([0, 350, 1000, 700]);
    // Rounded positions keep the total exact.
    const odd = distribute([r(0, 0, 10, 10), r(5, 0, 10, 10), r(101, 0, 10, 10)], "x");
    expect(odd.map((b) => b.x)).toEqual([0, 51, 101]);
    // Fewer than three: unchanged.
    expect(distribute([r(0, 0, 10, 10), r(50, 0, 10, 10)], "x").map((b) => b.x)).toEqual([0, 50]);
  });
});

describe("rulers and grid", () => {
  it("label steps are 1-2-5 multiples at least 56 CSS px apart", () => {
    expect(rulerStep(1)).toBe(100);
    expect(rulerStep(0.5)).toBe(200);
    expect(rulerStep(0.25)).toBe(500);
    expect(rulerStep(2)).toBe(50);
    expect(rulerStep(0)).toBe(0);
  });

  it("one path of ticks; labels at every step", () => {
    const t = rulerTicks(1920, 0.5, 20);
    expect(t.labels).toEqual([0, 200, 400, 600, 800, 1000, 1200, 1400, 1600, 1800]);
    // Minor ticks every 20 px (10 CSS px): 97 marks for 0..1920.
    expect(t.path.match(/M/g)).toHaveLength(97);
    expect(rulerTicks(0, 0.5, 20)).toEqual({ labels: [], path: "" });
  });

  it("draws the grid coarser when cells would be under 6 CSS px", () => {
    expect(visibleGrid(16, 1)).toBe(16);
    expect(visibleGrid(8, 0.5)).toBe(16);
    expect(visibleGrid(8, 0.25)).toBe(32);
    expect(visibleGrid(16, 0)).toBe(0);
  });
});

describe("editor commands", () => {
  function actions() {
    let v: ScreenEditorView = { ...DEFAULT_SCREEN_EDITOR };
    const a: EditorActions = {
      view: () => v,
      setView: (p) => (v = { ...v, ...p }),
      selectionSize: () => 0,
      layerCount: () => 2,
      hasGuides: () => false,
      selectAll: vi.fn(),
      align: vi.fn(),
      distribute: vi.fn(),
      clearGuides: vi.fn(),
      toggleLock: vi.fn(),
      editSafeAreas: vi.fn(),
    };
    return a;
  }

  it("cycles the crop overlay", () => {
    expect(nextCrop("none")).toBe("9:16");
    expect(nextCrop("9:16")).toBe("4:5");
    expect(nextCrop("4:3")).toBe("none");
  });

  it("toggles view options; align / distribute need a selection; shortcuts are unique", () => {
    const a = actions();
    const cmds = editorCommands(a);
    const by = (id: string) => cmds.find((c) => c.id === id)!;
    const ctx = currentContext();
    void by("editor.grid").run?.(ctx);
    expect(a.view().grid).toBe(true);
    void by("editor.safe").run?.(ctx);
    expect([a.view().actionSafe, a.view().titleSafe]).toEqual([true, true]);
    void by("editor.safe").run?.(ctx);
    expect([a.view().actionSafe, a.view().titleSafe]).toEqual([false, false]);
    void by("editor.crop").run?.(ctx);
    expect(a.view().crop).toBe("9:16");
    expect(by("editor.align.left").disabled?.(ctx)).toBe("Select a layer first");
    expect(by("editor.distribute.x").disabled?.(ctx)).toBe("Select three or more layers");
    expect(by("editor.clearGuides").disabled?.(ctx)).toBe("No guides");
    expect(by("editor.selectAll").disabled?.(ctx)).toBeNull();
    expect(by("editor.grid").defaultKeys).toEqual(["Ctrl+G"]);
    // Only in the layout editor, never while typing.
    expect(cmds.every((c) => c.keyScope === "screenEditor" && c.notWhileTyping)).toBe(true);
    expect(new Set(cmds.map((c) => c.id)).size).toBe(cmds.length);
    const keys = cmds.flatMap((c) => c.defaultKeys ?? []);
    expect(new Set(keys).size).toBe(keys.length);
  });
});

describe("shortcuts and the registry", () => {
  const key = (init: KeyboardEventInit) => new KeyboardEvent("keydown", init);

  afterEach(() => {
    for (const c of [...registry.list]) registerCommands([c])();
  });

  it("matches letters by physical key, punctuation by character", () => {
    expect(matchesShortcut(key({ key: "g", code: "KeyG", ctrlKey: true }), "Ctrl+G")).toBe(true);
    expect(matchesShortcut(key({ key: "G", code: "KeyG", ctrlKey: true, shiftKey: true }), "Ctrl+G")).toBe(false);
    expect(matchesShortcut(key({ key: "g", code: "KeyG", metaKey: true }), "Ctrl+G")).toBe(true);
    expect(matchesShortcut(key({ key: "g", code: "KeyG" }), "Ctrl+G")).toBe(false);
    expect(matchesShortcut(key({ key: "å", code: "KeyA", altKey: true }), "Alt+A")).toBe(true);
    expect(matchesShortcut(key({ key: "H", code: "KeyH", altKey: true, shiftKey: true }), "Alt+Shift+H")).toBe(true);
    // ";" needs Shift on many layouts (Norwegian: Shift+,), so Shift is ignored for punctuation.
    expect(matchesShortcut(key({ key: ";", code: "Comma", ctrlKey: true, shiftKey: true }), "Ctrl+;")).toBe(true);
    expect(matchesShortcut(key({ key: "'", code: "Backslash", ctrlKey: true }), "Ctrl+'")).toBe(true);
    expect(matchesShortcut(key({ key: "'", code: "Backslash", ctrlKey: true, altKey: true }), "Ctrl+'")).toBe(false);
  });

  it("a key bound twice runs nothing (a conflict); never while typing; unregister removes it", () => {
    const first = vi.fn();
    const second = vi.fn();
    const off1 = registerCommands([{ id: "a", title: "A", section: "settings", defaultKeys: ["Ctrl+G"], notWhileTyping: true, run: first }]);
    const off2 = registerCommands([{ id: "b", title: "B", section: "settings", defaultKeys: ["Ctrl+G"], notWhileTyping: true, run: second }]);
    expect(runShortcut(key({ key: "g", code: "KeyG", ctrlKey: true }))).toBe(true);
    expect(second).not.toHaveBeenCalled();
    expect(first).not.toHaveBeenCalled();

    off2();
    const input = document.createElement("input");
    document.body.append(input);
    const typed = key({ key: "g", code: "KeyG", ctrlKey: true });
    Object.defineProperty(typed, "target", { value: input });
    expect(runShortcut(typed)).toBe(false);
    input.remove();

    expect(runShortcut(key({ key: "g", code: "KeyG", ctrlKey: true }))).toBe(true);
    expect(first).toHaveBeenCalledTimes(1);
    off1();
    expect(runShortcut(key({ key: "g", code: "KeyG", ctrlKey: true }))).toBe(false);
  });

  it("disabled commands don't run", () => {
    const run = vi.fn();
    registerCommands([{ id: "x", title: "X", section: "settings", defaultKeys: ["Alt+A"], run, disabled: () => "No" }]);
    expect(runShortcut(key({ key: "a", code: "KeyA", altKey: true }))).toBe(false);
    expect(run).not.toHaveBeenCalled();
  });
});

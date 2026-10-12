import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ACTION_SAFE,
  FrameQueue,
  aspectLock,
  dragKeepsAspect,
  graphicRatio,
  scalesOnResize,
  TITLE_SAFE,
  Throttle,
  boundsOf,
  clampRect,
  cropRect,
  fitStage,
  moveGroup,
  moveRect,
  nudge,
  READOUT_W,
  readoutPlace,
  readoutText,
  resizeRect,
  resizeSnap,
  safeRect,
  sizeRect,
  snapMove,
  snapTargets,
  snapThreshold,
  toCanvas,
  toScreen,
} from "./geometry";

const HD = { w: 1920, h: 1080 };

describe("scaling", () => {
  it("fits the screen into the area, keeping its aspect ratio", () => {
    expect(fitStage(960, 1000, 1920, 1080)).toEqual({ w: 960, h: 540, scale: 0.5 });
    // Height-bound.
    const s = fitStage(2000, 540, 1920, 1080);
    expect(s.scale).toBe(0.5);
    expect(s.w).toBe(960);
    // Vertical screen.
    expect(fitStage(1000, 960, 1080, 1920).scale).toBe(0.5);
  });

  it("has no size until the area is measured", () => {
    expect(fitStage(0, 0, 1920, 1080)).toEqual({ w: 0, h: 0, scale: 0 });
    expect(toScreen(10, 0)).toBe(0);
    expect(snapThreshold(0)).toBe(0);
  });

  it("converts between screen and canvas px", () => {
    expect(toCanvas({ x: 100, y: 50, w: 400, h: 200 }, 0.5)).toEqual({ x: 50, y: 25, w: 200, h: 100 });
    expect(toScreen(30, 0.5)).toBe(60);
    // ~6 CSS px on a half-size stage is 12 screen px.
    expect(snapThreshold(0.5)).toBe(12);
    expect(snapThreshold(0.25, 6)).toBe(24);
  });
});

describe("move and resize", () => {
  const start = { x: 100, y: 100, w: 400, h: 200 };

  it("moves by the delta", () => {
    expect(moveRect(start, 30, -20)).toEqual({ x: 130, y: 80, w: 400, h: 200 });
  });

  it("resizes from each handle, keeping the opposite side", () => {
    expect(resizeRect(start, "e", 50, 999)).toEqual({ x: 100, y: 100, w: 450, h: 200 });
    expect(resizeRect(start, "w", 50, 0)).toEqual({ x: 150, y: 100, w: 350, h: 200 });
    expect(resizeRect(start, "n", 0, -40)).toEqual({ x: 100, y: 60, w: 400, h: 240 });
    expect(resizeRect(start, "s", 0, 40)).toEqual({ x: 100, y: 100, w: 400, h: 240 });
    expect(resizeRect(start, "se", 10, 20)).toEqual({ x: 100, y: 100, w: 410, h: 220 });
    expect(resizeRect(start, "nw", 10, 20)).toEqual({ x: 110, y: 120, w: 390, h: 180 });
  });

  it("never goes below the minimum size (the edge stops at the other one)", () => {
    expect(resizeRect(start, "e", -1000, 0)).toEqual({ x: 100, y: 100, w: 1, h: 200 });
    expect(resizeRect(start, "w", 1000, 0)).toEqual({ x: 499, y: 100, w: 1, h: 200 });
    expect(resizeRect(start, "n", 0, 1000)).toMatchObject({ y: 299, h: 1 });
  });

  it("Shift keeps the aspect ratio: corners follow the larger change", () => {
    // 2:1 box; the width grows 50 %, the height follows.
    const r = resizeRect(start, "se", 200, 10, true);
    expect(r).toEqual({ x: 100, y: 100, w: 600, h: 300 });
    // The height grows more: the width follows.
    const r2 = resizeRect(start, "se", 0, 100, true);
    expect(r2).toEqual({ x: 100, y: 100, w: 600, h: 300 });
    // A top-left corner anchors the bottom-right.
    const r3 = resizeRect(start, "nw", -200, 0, true);
    expect(r3.x + r3.w).toBe(500);
    expect(r3.y + r3.h).toBe(300);
    expect(r3.w / r3.h).toBeCloseTo(2);
  });

  it("Shift on an edge handle grows the other axis around the centre", () => {
    const r = resizeRect(start, "e", 400, 0, true);
    expect(r.w).toBe(800);
    expect(r.h).toBe(400);
    expect(r.x).toBe(100);
    expect(r.y + r.h / 2).toBe(200); // centre kept
  });

  it("clamps to integers, >= 1 px and at least 1 px on screen", () => {
    expect(clampRect({ x: 10.4, y: 9.6, w: 99.5, h: 0.2 }, HD)).toEqual({ x: 10, y: 10, w: 100, h: 1 });
    expect(clampRect({ x: -5000, y: 5000, w: 100, h: 100 }, HD)).toEqual({ x: -99, y: 1079, w: 100, h: 100 });
    expect(clampRect({ x: 3000, y: -3000, w: 100, h: 100 }, HD)).toEqual({ x: 1919, y: -99, w: 100, h: 100 });
  });
});

describe("snapping", () => {
  const targets = snapTargets(HD, [{ x: 300, y: 400, w: 200, h: 100 }]);

  it("targets: edges, centre, action / title safe (93 % / 90 %), other layers' edges and centres", () => {
    // Action safe: 3.5 % in from each side (67.2 / 37.8 → 67 / 38); title safe: 5 % (96 / 54).
    expect(targets.x).toEqual(expect.arrayContaining([0, 960, 1920, 67, 1853, 96, 1824, 300, 400, 500]));
    expect(targets.y).toEqual(expect.arrayContaining([0, 540, 1080, 38, 1042, 54, 1026, 400, 450, 500]));
    // The old 10 % line is gone; lists are sorted and unique.
    expect(targets.x).not.toContain(192);
    expect(targets.x).toEqual([...new Set(targets.x)].sort((a, b) => a - b));
  });

  it("SMPTE ST 2046-1 safe areas: action safe the central 93 %, title safe the central 90 %", () => {
    expect(ACTION_SAFE).toBe(0.93);
    expect(TITLE_SAFE).toBe(0.9);
    const a = safeRect(HD, ACTION_SAFE);
    expect(a.x).toBeCloseTo(67.2);
    expect(a.y).toBeCloseTo(37.8);
    expect(a.w).toBeCloseTo(1785.6);
    expect(a.h).toBeCloseTo(1004.4);
    expect(safeRect(HD, TITLE_SAFE)).toEqual({ x: 96, y: 54, w: 1728, h: 972 });
  });

  it("snaps a moving box's nearest edge or centre and reports a guide", () => {
    // Left edge 5 px off the title safe line (96).
    const { rect, guides } = snapMove({ x: 101, y: 700, w: 150, h: 50 }, targets, 12);
    expect(rect.x).toBe(96);
    expect(guides).toContainEqual({ axis: "x", pos: 96 });
    // Centre near the screen centre.
    const c = snapMove({ x: 905, y: 515, w: 100, h: 50 }, targets, 12);
    expect(c.rect.x + c.rect.w / 2).toBe(960);
    expect(c.rect.y + c.rect.h / 2).toBe(540);
    expect(c.guides).toEqual([
      { axis: "x", pos: 960 },
      { axis: "y", pos: 540 },
    ]);
  });

  it("snaps to another layer's edge and centre", () => {
    const { rect } = snapMove({ x: 505, y: 700, w: 100, h: 50 }, targets, 12);
    expect(rect.x).toBe(500);
    // Centre (x + 25) near the other layer's centre (400).
    expect(snapMove({ x: 370, y: 700, w: 50, h: 50 }, targets, 12).rect.x).toBe(375);
  });

  it("snaps to ruler guides, custom safe areas, the crop and the grid", () => {
    const crop = cropRect(HD, "9:16")!;
    const t = snapTargets(HD, [], {
      x: [700],
      y: [333],
      rects: [{ x: 1500, y: 200, w: 100, h: 100 }, crop],
    });
    const tg = { ...t, grid: 16 };
    expect(snapMove({ x: 704, y: 900, w: 10, h: 10 }, t, 6).rect.x).toBe(700);
    expect(snapMove({ x: 1100, y: 330, w: 10, h: 100 }, t, 6).rect.y).toBe(333);
    // Custom safe area: left edge and centre (1550).
    expect(snapMove({ x: 1497, y: 900, w: 100, h: 10 }, t, 6).rect.x).toBe(1500);
    expect(snapMove({ x: 1543, y: 900, w: 10, h: 10 }, t, 6).rect.x).toBe(1545);
    // 9:16 crop of 1920×1080: 607.5 wide, centred: edges 656.25 / 1263.75 → 656 / 1264.
    expect(snapMove({ x: 659, y: 900, w: 10, h: 10 }, t, 6).rect.x).toBe(656);
    // Grid: 16 px multiples, edges or centre.
    const g = snapMove({ x: 1110, y: 870, w: 30, h: 30 }, tg, 6);
    // Left 1110 is 6 from 1104, the centre 1125 is 5 from 1120, the right 1140 is 4 from 1136: the right edge wins.
    expect(g.rect.x).toBe(1106);
    expect(g.guides).toContainEqual({ axis: "x", pos: 1136 });
    // A tie between a line and the grid goes to the line.
    expect(snapMove({ x: 702, y: 900, w: 10, h: 10 }, tg, 6).rect.x).toBe(700);
    // Nothing without the grid.
    expect(snapMove({ x: 1110, y: 870, w: 30, h: 30 }, snapTargets(HD, []), 6).rect.x).toBe(1110);
  });

  it("the threshold is 6 CSS px at any zoom, converted to screen px; Alt (0) disables it", () => {
    expect(snapThreshold(0.5)).toBe(12);
    expect(snapThreshold(1)).toBe(6);
    expect(snapThreshold(2)).toBe(3);
    const t = snapTargets(HD, [], { x: [700] });
    const box = { x: 710, y: 900, w: 10, h: 10 }; // 10 screen px off the guide
    expect(snapMove(box, t, snapThreshold(0.5)).rect.x).toBe(700); // 5 CSS px on screen
    expect(snapMove(box, t, snapThreshold(1)).rect.x).toBe(710); // 10 CSS px: too far
    expect(snapMove(box, t, snapThreshold(2)).rect.x).toBe(710);
    expect(snapMove({ ...box, x: 702 }, t, snapThreshold(2)).rect.x).toBe(700);
    expect(snapMove({ ...box, x: 702 }, t, 0).rect.x).toBe(702);
  });

  it("crop overlays: the largest centred box of each aspect ratio", () => {
    expect(cropRect(HD, "none")).toBeNull();
    expect(cropRect(HD, "1:1")).toEqual({ x: 420, y: 0, w: 1080, h: 1080 });
    expect(cropRect(HD, "4:3")).toEqual({ x: 240, y: 0, w: 1440, h: 1080 });
    expect(cropRect(HD, "4:5")).toEqual({ x: 528, y: 0, w: 864, h: 1080 });
    expect(cropRect(HD, "9:16")).toEqual({ x: 656.25, y: 0, w: 607.5, h: 1080 });
    // A portrait screen: 4:3 is limited by the width.
    expect(cropRect({ w: 1080, h: 1920 }, "4:3")).toEqual({ x: 0, y: 555, w: 1080, h: 810 });
  });

  it("doesn't snap beyond the threshold, or with a zero threshold (Alt)", () => {
    expect(snapMove({ x: 220, y: 700, w: 100, h: 50 }, targets, 12)).toEqual({
      rect: { x: 220, y: 700, w: 100, h: 50 },
      guides: [],
    });
    expect(snapMove({ x: 197, y: 700, w: 100, h: 50 }, targets, 0).rect.x).toBe(197);
  });

  it("resize snaps only the moving edges", () => {
    const start = { x: 197, y: 700, w: 1700, h: 50 };
    // Right edge dragged to 1917, near 1920.
    const e = resizeSnap(start, "e", 20, 0, {}, targets, 12);
    expect(e.rect).toEqual({ x: 197, y: 700, w: 1723, h: 50 }); // left edge untouched
    expect(e.guides).toEqual([{ axis: "x", pos: 1920 }]);
    // Left edge dragged to 101, near 96 (title safe); the right edge stays at 1917.
    const w = resizeSnap({ x: 120, y: 700, w: 1797, h: 50 }, "w", -19, 0, {}, targets, 12);
    expect(w.rect.x).toBe(96);
    expect(w.rect.x + w.rect.w).toBe(1917);
    // A zero threshold doesn't snap.
    expect(resizeSnap(start, "e", 20, 0, {}, targets, 0).rect.w).toBe(1720);
  });
});

describe("drag readout", () => {
  it("shows X / Y while moving and W / H while resizing, in whole screen px", () => {
    const r = { x: 120, y: 840, w: 640, h: 180 };
    expect(readoutText("move", r)).toBe("X 120 Y 840");
    expect(readoutText("size", r)).toBe("W 640 H 180");
    expect(readoutText("move", { x: -12.4, y: 0.6, w: 1, h: 1 })).toBe("X -12 Y 1");
  });

  it("sits under the box's bottom-left corner when there is room", () => {
    // Stage 960×540 at scale 0.5.
    expect(readoutPlace({ x: 100, y: 100, w: 400, h: 200 }, HD, 0.5)).toEqual({ x: 50, y: 154, tx: 0, ty: 0 });
  });

  it("goes over the box near the bottom, inside it when the box fills the height", () => {
    expect(readoutPlace({ x: 100, y: 900, w: 400, h: 180 }, HD, 0.5)).toEqual({ x: 50, y: 446, tx: 0, ty: -100 });
    expect(readoutPlace({ x: 0, y: 0, w: 1920, h: 1080 }, HD, 0.5)).toEqual({ x: 0, y: 536, tx: 0, ty: -100 });
  });

  it("stays on the stage horizontally", () => {
    // Left edge off-screen: clamped to 0.
    expect(readoutPlace({ x: -300, y: 100, w: 400, h: 100 }, HD, 0.5).x).toBe(0);
    // Too close to the right edge: right-aligned to the box (or the stage edge).
    expect(readoutPlace({ x: 1800, y: 100, w: 100, h: 100 }, HD, 0.5)).toMatchObject({ x: 950, tx: -100 });
    expect(readoutPlace({ x: 1900, y: 100, w: 400, h: 100 }, HD, 0.5)).toMatchObject({ x: 960, tx: -100 });
    // Right-aligned, it never starts left of the stage.
    expect(readoutPlace({ x: 10, y: 10, w: 10, h: 10 }, { w: 100, h: 100 }, 0.5)).toMatchObject({ x: READOUT_W, tx: -100 });
  });
});

describe("nudge", () => {
  const r = { x: 10, y: 20, w: 30, h: 40 };
  it("moves 1 px, or 10 px with Shift", () => {
    expect(nudge(r, "ArrowLeft", false)).toEqual({ ...r, x: 9 });
    expect(nudge(r, "ArrowRight", true)).toEqual({ ...r, x: 20 });
    expect(nudge(r, "ArrowUp", false)).toEqual({ ...r, y: 19 });
    expect(nudge(r, "ArrowDown", true)).toEqual({ ...r, y: 30 });
    expect(nudge(r, "a", false)).toBeNull();
  });
});

describe("timing", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => setTimeout(() => cb(0), 16) as unknown as number);
    vi.stubGlobal("cancelAnimationFrame", (id: number) => clearTimeout(id));
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("Throttle: <= 1 call per interval, the trailing value always delivered", () => {
    const fn = vi.fn();
    const t = new Throttle<number>(50, fn);
    // A 1 s drag with a pointer move every 4 ms (250 moves).
    for (let i = 0; i < 250; i++) {
      t.call(i);
      vi.advanceTimersByTime(4);
    }
    expect(fn.mock.calls.length).toBeLessThanOrEqual(21);
    expect(fn.mock.calls.length).toBeGreaterThanOrEqual(19);
    vi.advanceTimersByTime(100);
    expect(fn.mock.calls.at(-1)?.[0]).toBe(249);
    // Calls are >= 50 ms apart.
    const times: number[] = [];
    const t2 = new Throttle<number>(50, () => times.push(Date.now()));
    for (let i = 0; i < 100; i++) {
      t2.call(i);
      vi.advanceTimersByTime(7);
    }
    for (let i = 1; i < times.length; i++) expect(times[i] - times[i - 1]).toBeGreaterThanOrEqual(50);
  });

  it("Throttle: flush delivers the pending value now; cancel drops it; nothing runs at rest", () => {
    const fn = vi.fn();
    const t = new Throttle<string>(50, fn);
    t.call("a");
    t.call("b");
    expect(fn.mock.calls).toEqual([["a"]]);
    t.flush();
    expect(fn.mock.calls).toEqual([["a"], ["b"]]);
    t.call("c");
    t.cancel();
    vi.advanceTimersByTime(1000);
    expect(fn).toHaveBeenCalledTimes(2);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("FrameQueue: one callback per frame with the latest value", () => {
    const fn = vi.fn();
    const q = new FrameQueue<number>(fn);
    q.push(1);
    q.push(2);
    q.push(3);
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(16);
    expect(fn.mock.calls).toEqual([[3]]);
    expect(vi.getTimerCount()).toBe(0);
    q.push(4);
    q.flush();
    expect(fn.mock.calls).toEqual([[3], [4]]);
    q.push(5);
    q.cancel();
    vi.advanceTimersByTime(100);
    expect(fn).toHaveBeenCalledTimes(2);
  });
});

describe("groups", () => {
  it("bounds of a selection", () => {
    expect(boundsOf([])).toBeNull();
    expect(
      boundsOf([
        { x: 100, y: 50, w: 200, h: 100 },
        { x: -20, y: 300, w: 50, h: 10 },
      ]),
    ).toEqual({ x: -20, y: 50, w: 320, h: 260 });
  });

  it("moves every box by the same whole-pixel delta, limited so each keeps a pixel on screen", () => {
    const starts = [
      { x: 100, y: 100, w: 200, h: 100 },
      { x: 1700, y: 900, w: 100, h: 100 },
    ];
    expect(moveGroup(starts, 10.4, -20.6, HD)).toEqual([
      { x: 110, y: 79, w: 200, h: 100 },
      { x: 1710, y: 879, w: 100, h: 100 },
    ]);
    // Far right: the second box stops at x = 1919; the first moves the same and keeps its shape.
    expect(moveGroup(starts, 5000, 0, HD)).toEqual([
      { x: 319, y: 100, w: 200, h: 100 },
      { x: 1919, y: 900, w: 100, h: 100 },
    ]);
    // Far left: the first box keeps one pixel (x = 1 - w).
    expect(moveGroup(starts, -5000, 0, HD)[0].x).toBe(-199);
  });
});

describe("resize: from the centre, snapping, minimum", () => {
  const start = { x: 100, y: 100, w: 400, h: 200 }; // centre 300, 200
  const none = { x: [], y: [] };

  it("Alt mirrors the drag on the opposite edge (the centre stays put)", () => {
    expect(resizeRect(start, "e", 50, 0, false, 1, true)).toEqual({ x: 50, y: 100, w: 500, h: 200 });
    expect(resizeRect(start, "w", -50, 0, false, 1, true)).toEqual({ x: 50, y: 100, w: 500, h: 200 });
    expect(resizeRect(start, "n", 0, 20, false, 1, true)).toEqual({ x: 100, y: 120, w: 400, h: 160 });
    const c = resizeRect(start, "se", 50, 30, false, 1, true);
    expect(c).toEqual({ x: 50, y: 70, w: 500, h: 260 });
  });

  it("Alt with Shift keeps the ratio around the centre", () => {
    const r = resizeRect(start, "se", 100, 0, true, 1, true);
    expect(r).toEqual({ x: 0, y: 50, w: 600, h: 300 });
    const e = resizeRect(start, "w", -100, 0, true, 1, true);
    expect(e).toEqual({ x: 0, y: 50, w: 600, h: 300 });
  });

  it("never goes below the minimum, from the centre or with the aspect lock", () => {
    expect(resizeRect(start, "e", -1000, 0, false, 16)).toEqual({ x: 100, y: 100, w: 16, h: 200 });
    expect(resizeRect(start, "w", 1000, 0, false, 16)).toEqual({ x: 484, y: 100, w: 16, h: 200 });
    expect(resizeRect(start, "e", -1000, 0, false, 16, true)).toEqual({ x: 292, y: 100, w: 16, h: 200 });
    // 2:1 with the lock: the height reaches 16 first, the width follows at 32.
    const k = resizeRect(start, "se", -1000, -1000, true, 16);
    expect(k).toEqual({ x: 100, y: 100, w: 32, h: 16 });
    // A box already smaller than the minimum keeps its size as the floor.
    expect(resizeRect({ x: 0, y: 0, w: 10, h: 10 }, "e", -100, 0, false, 16)).toEqual({ x: 0, y: 0, w: 10, h: 10 });
    expect(resizeRect({ x: 0, y: 0, w: 10, h: 10 }, "e", 20, 0, false, 16).w).toBe(30);
  });

  it("from the centre, either mirrored edge snaps", () => {
    const t = snapTargets(HD, [], { x: [560] }); // also 0, 960, 1920, safe areas...
    // Centre 300; dragging the right edge to 543 puts the left one at 57: 57 is near 67.2 (action safe)? no;
    // use a guide the RIGHT edge reaches: 556 → 560.
    const r = resizeSnap(start, "e", 56, 0, { fromCenter: true }, t, 6);
    expect(r.rect).toEqual({ x: 40, y: 100, w: 520, h: 200 });
    expect(r.guides).toEqual([{ axis: "x", pos: 560 }]);
    // The mirrored LEFT edge reaching a target: left at 2 → snaps to 0 (screen edge).
    const l = resizeSnap(start, "e", 98, 0, { fromCenter: true }, snapTargets(HD, []), 6);
    expect(l.rect.x).toBe(0);
    expect(l.rect.w).toBe(600);
    expect(l.guides).toEqual([{ axis: "x", pos: 0 }]);
  });

  it("with the aspect lock, the nearest snap of either axis drives and the other side follows", () => {
    const t = { x: [705], y: [] as number[] };
    // se corner by (300, 150): 700×350 at 2:1; the right edge 800 isn't near 705 → no snap.
    expect(resizeSnap(start, "se", 300, 150, { keepAspect: true }, t, 6).guides).toEqual([]);
    // se by (203, 0): w 603 → right edge 703, 2 from 705 → w 605, h 302.5.
    const s = resizeSnap(start, "se", 203, 0, { keepAspect: true }, { x: [705], y: [] }, 6);
    expect(s.rect).toEqual({ x: 100, y: 100, w: 605, h: 302.5 });
    expect(s.guides).toEqual([{ axis: "x", pos: 705 }]);
    // The derived axis can snap too: bottom edge at 100 + 302.5 = 402.5 near 400 → h 300, w 600.
    const y = resizeSnap(start, "se", 205, 0, { keepAspect: true }, { x: [], y: [400] }, 6);
    expect(y.rect).toEqual({ x: 100, y: 100, w: 600, h: 300 });
    expect(y.guides).toEqual([{ axis: "y", pos: 400 }]);
    // Ratio kept in every case.
    expect(s.rect.w / s.rect.h).toBeCloseTo(2);
  });

  it("a snap that would go below the minimum is ignored", () => {
    const r = resizeSnap(start, "e", -385, 0, { min: 16 }, { x: [110], y: [] }, 6);
    // Raw width 15 → 16 (min); the target 110 would make it 10: ignored.
    expect(r.rect.w).toBe(16);
    expect(r.guides).toEqual([]);
    expect(resizeSnap(start, "e", 0, 0, {}, none, 6).rect).toEqual(start);
  });
});

describe("typed W / H", () => {
  const cur = { x: 10, y: 20, w: 400, h: 200 };

  it("sets the side, at least the minimum", () => {
    expect(sizeRect(cur, { w: 500 }, false, 16)).toEqual({ x: 10, y: 20, w: 500, h: 200 });
    expect(sizeRect(cur, { w: 3 }, false, 16)).toEqual({ x: 10, y: 20, w: 16, h: 200 });
    expect(sizeRect(cur, { x: 50 }, true, 16)).toEqual({ x: 50, y: 20, w: 400, h: 200 });
  });

  it("with the aspect lock the other side follows, both at least the minimum", () => {
    expect(sizeRect(cur, { w: 600 }, true, 16)).toEqual({ x: 10, y: 20, w: 600, h: 300 });
    expect(sizeRect(cur, { h: 50 }, true, 16)).toEqual({ x: 10, y: 20, w: 100, h: 50 });
    // W 20 at 2:1 would make H 10: both grow so H is 16.
    expect(sizeRect(cur, { w: 20 }, true, 16)).toEqual({ x: 10, y: 20, w: 32, h: 16 });
  });
});

// ---- Resizing scales ------------------------------------------------------------------------------

describe("resizing scales", () => {
  it("every layer but an explicit Reflow one scales; its lock is on by default", () => {
    expect(scalesOnResize(undefined)).toBe(true);
    expect(scalesOnResize(null)).toBe(true);
    expect(scalesOnResize("fit")).toBe(true);
    expect(scalesOnResize("reflow")).toBe(false);
    expect(aspectLock(undefined, undefined)).toBe(true);
    expect(aspectLock("fit", undefined)).toBe(true);
    expect(aspectLock("reflow", undefined)).toBe(false);
    // A toggled lock wins.
    expect(aspectLock("fit", false)).toBe(false);
    expect(aspectLock("reflow", true)).toBe(true);
  });

  it("a scaling layer keeps its aspect unless Shift; a Reflow one only with Shift or the lock", () => {
    expect(dragKeepsAspect(true, true, false)).toBe(true);
    expect(dragKeepsAspect(true, true, true)).toBe(false);
    // Unlocked scaling layer: Shift keeps it.
    expect(dragKeepsAspect(true, false, false)).toBe(false);
    expect(dragKeepsAspect(true, false, true)).toBe(true);
    expect(dragKeepsAspect(false, false, false)).toBe(false);
    expect(dragKeepsAspect(false, false, true)).toBe(true);
    expect(dragKeepsAspect(false, true, true)).toBe(true);
  });

  it("the graphic's aspect: the design box, else the layer's own box", () => {
    expect(graphicRatio({ x: 0, y: 0, w: 1920, h: 1080 }, { w: 1080, h: 230 })).toBeCloseTo(1080 / 230);
    expect(graphicRatio({ x: 0, y: 0, w: 800, h: 200 }, null)).toBe(4);
    expect(graphicRatio({ x: 0, y: 0, w: 800, h: 200 }, { w: 0, h: 0 })).toBe(4);
  });

  it("a resize keeps the given ratio: a box of another shape takes the design's on the first move", () => {
    // A square reflow box of a 4:1 graphic, its se corner dragged onto the 4:1 diagonal.
    expect(resizeSnap({ x: 0, y: 0, w: 1000, h: 1000 }, "se", -200, -800, { keepAspect: true, ratio: 4 }).rect).toEqual({ x: 0, y: 0, w: 800, h: 200 });
    // Off the diagonal the corner follows the pointer's projection onto it: a full-screen layer
    // dragged half-way left (barely up) ends about half as wide, at 4:1, not a huge box.
    const r = resizeSnap({ x: 0, y: 0, w: 1920, h: 1080 }, "se", -960, -10, { keepAspect: true, ratio: 4 }).rect;
    expect(r.w / r.h).toBeCloseTo(4);
    expect(r.w).toBeCloseTo((960 * 4 + 1070) / 17 * 4, 3);
    expect(r.w).toBeLessThan(1920);
    // An edge handle grows the other side around the centre.
    const e = resizeSnap({ x: 100, y: 100, w: 400, h: 100 }, "e", 400, 0, { keepAspect: true, ratio: 4 }).rect;
    expect(e).toEqual({ x: 100, y: 50, w: 800, h: 200 });
    // Snapping still applies: the right edge snaps to 1000 and the height follows.
    const s = resizeSnap({ x: 100, y: 100, w: 400, h: 100 }, "se", 497, 0, { keepAspect: true, ratio: 4 }, { x: [1000], y: [] }, 6);
    expect(s.rect).toEqual({ x: 100, y: 100, w: 900, h: 225 });
    expect(s.guides).toEqual([{ axis: "x", pos: 1000 }]);
  });

  it("a typed W / H keeps the given ratio while locked", () => {
    const cur = { x: 10, y: 20, w: 500, h: 500 };
    expect(sizeRect(cur, { w: 700 }, true, 16, 1400 / 260)).toEqual({ x: 10, y: 20, w: 700, h: 130 });
    expect(sizeRect(cur, { h: 130 }, true, 16, 1400 / 260)).toEqual({ x: 10, y: 20, w: 700, h: 130 });
    // Unlocked: only the typed side.
    expect(sizeRect(cur, { w: 700 }, false, 16, 1400 / 260)).toEqual({ x: 10, y: 20, w: 700, h: 500 });
  });
});

import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { LayerAnchor, ScreenDoc as AnyScreenDoc, TemplateScreenLayer as ScreenLayer } from "$lib/types";
import { anchorMargin, movedAnchor, normalizeAnchor, placeAnchor, setAnchors } from "./anchor";
import { anchorLabel, commonAnchor } from "./anchor-pad";
import AnchorPad from "./AnchorPad.svelte";
import { setRects } from "./model";
/** Test docs hold template layers only. */
type ScreenDoc = Omit<AnyScreenDoc, "layers"> & { layers: ScreenLayer[] };

const SCREEN = { width: 1920, height: 1080 };
const r = (x: number, y: number, w: number, h: number) => ({ x, y, w, h });
const a = (h: LayerAnchor["h"], v: LayerAnchor["v"], safe = false): LayerAnchor => ({ h, v, safe });

// The same cases as core/anchors.rs (the core is the authority; these must agree).
describe("placement math (mirror of core/anchors.rs)", () => {
  it("margins are 5 % rounded inside title safe, else 0", () => {
    expect(anchorMargin(1920, true)).toBe(96);
    expect(anchorMargin(1080, true)).toBe(54);
    expect(anchorMargin(1280, true)).toBe(64);
    expect(anchorMargin(720, true)).toBe(36);
    expect(anchorMargin(1920, false)).toBe(0);
  });

  it("places all nine points with and without safe", () => {
    const b = r(5, 7, 400, 100);
    const hs = [
      ["left", 0, 96],
      ["center", 760, 760],
      ["right", 1520, 1424],
    ] as const;
    const vs = [
      ["top", 0, 54],
      ["middle", 490, 490],
      ["bottom", 980, 926],
    ] as const;
    for (const [h, x0, xs] of hs)
      for (const [v, y0, ys] of vs) {
        expect(placeAnchor(b, a(h, v), SCREEN)).toEqual(r(x0, y0, 400, 100));
        expect(placeAnchor(b, a(h, v, true), SCREEN)).toEqual(r(xs, ys, 400, 100));
      }
  });

  it("a single axis keeps the other coordinate; halves round up; big boxes follow the formula", () => {
    expect(placeAnchor(r(5, 7, 400, 100), a("center", null), SCREEN)).toEqual(r(760, 7, 400, 100));
    expect(placeAnchor(r(5, 7, 400, 100), a(null, "bottom", true), SCREEN)).toEqual(r(5, 926, 400, 100));
    expect(placeAnchor(r(0, 0, 401, 1), a("center", null), SCREEN).x).toBe(760);
    expect(placeAnchor(r(0, 0, 2001, 1), a("center", null), SCREEN).x).toBe(-40);
    expect(placeAnchor(r(0, 0, 3000, 2000), a("right", "bottom", true), SCREEN)).toEqual(
      r(1824 - 3000, 1026 - 2000, 3000, 2000),
    );
  });

  it("normalizes an empty anchor away", () => {
    expect(normalizeAnchor(a(null, null))).toBeNull();
    expect(normalizeAnchor(undefined)).toBeNull();
    expect(normalizeAnchor(a(null, null, true))).toEqual(a(null, null, true));
  });

  it("a move releases the moved axis; a resize keeps the anchor", () => {
    const c = a("center", "middle", true);
    expect(movedAnchor(r(760, 490, 400, 100), c, r(800, 490, 400, 100), SCREEN)).toEqual({
      rect: r(800, 490, 400, 100),
      anchor: a(null, "middle", true),
    });
    expect(movedAnchor(r(760, 490, 400, 100), c, r(760, 10, 400, 100), SCREEN).anchor).toEqual(a("center", null, true));
    expect(movedAnchor(r(760, 490, 400, 100), c, r(1, 2, 400, 100), SCREEN)).toEqual({ rect: r(1, 2, 400, 100), anchor: null });
    const cb = a("center", "bottom");
    expect(movedAnchor(r(760, 980, 400, 100), cb, r(760, 980, 600, 100), SCREEN)).toEqual({
      rect: r(660, 980, 600, 100),
      anchor: cb,
    });
    expect(movedAnchor(r(760, 980, 400, 100), cb, r(700, 900, 460, 180), SCREEN).rect).toEqual(r(730, 900, 460, 180));
  });

  it("labels the grid cells", () => {
    expect(anchorLabel("left", "top")).toBe("Top left");
    expect(anchorLabel("center", "middle")).toBe("Center");
    expect(anchorLabel("right", "bottom")).toBe("Bottom right");
  });
});

const layer = (id: string, over: Partial<ScreenLayer> = {}): ScreenLayer => ({
  id,
  name: id,
  kind: "template",
  templateId: "t",
  rect: r(100, 100, 400, 200),
  fields: {},
  vars: {},
  ...over,
});
const doc = (layers: ScreenLayer[]): ScreenDoc => ({
  id: "s",
  name: "S",
  width: 1920,
  height: 1080,
  background: "transparent",
  layers,
  updatedAt: "",
});

describe("document edits", () => {
  it("setAnchors places each layer on its own; free drops the key", () => {
    const d = doc([layer("a"), layer("b", { rect: r(0, 0, 100, 50), anchor: a(null, "top") }), layer("c")]);
    const next = setAnchors(d, ["a", "b"], { h: "center", v: "middle" });
    expect(next.layers[0]).toMatchObject({ anchor: a("center", "middle"), rect: r(760, 440, 400, 200) });
    expect(next.layers[1]).toMatchObject({ anchor: a("center", "middle"), rect: r(910, 515, 100, 50) });
    expect(next.layers[2]).toBe(d.layers[2]);
    // One axis on top of each layer's own.
    const h = setAnchors(d, ["a", "b"], { h: "right" });
    expect(h.layers[0].anchor).toEqual(a("right", null));
    expect(h.layers[1]).toMatchObject({ anchor: a("right", "top"), rect: r(1820, 0, 100, 50) });
    const free = setAnchors(next, ["a"], { h: null, v: null });
    expect("anchor" in free.layers[0]).toBe(false);
    expect(free.layers[0].rect).toEqual(r(760, 440, 400, 200));
  });

  it("setRects: a move releases the moved axis, a resize stays anchored, free layers move as before", () => {
    const d = doc([layer("a", { rect: r(760, 440, 400, 200), anchor: a("center", "middle") }), layer("b")]);
    const moved = setRects(d, [
      { id: "a", rect: r(700, 440, 400, 200) },
      { id: "b", rect: r(1, 2, 400, 200) },
    ]);
    expect(moved.layers[0]).toMatchObject({ rect: r(700, 440, 400, 200), anchor: a(null, "middle") });
    expect(moved.layers[1].rect).toEqual(r(1, 2, 400, 200));
    expect("anchor" in moved.layers[1]).toBe(false);
    const resized = setRects(d, [{ id: "a", rect: r(760, 440, 600, 200) }]);
    expect(resized.layers[0]).toMatchObject({ rect: r(660, 440, 600, 200), anchor: a("center", "middle") });
  });

  it("commonAnchor: shared, free or mixed", () => {
    expect(commonAnchor([layer("a"), layer("b")])).toBeNull();
    expect(commonAnchor([layer("a", { anchor: a("left", "top") }), layer("b", { anchor: a("left", "top") })])).toEqual(
      a("left", "top"),
    );
    expect(commonAnchor([layer("a", { anchor: a("left", "top") }), layer("b")])).toBe("mixed");
  });
});

describe("AnchorPad", () => {
  afterEach(() => cleanup());

  function mount(anchor: LayerAnchor | null | "mixed") {
    const onanchor = vi.fn();
    const r = render(AnchorPad, { props: { anchor, onanchor } });
    const cell = (name: string) => screen.getByRole("button", { name }) as HTMLButtonElement;
    return { ...r, onanchor, cell };
  }

  it("free: nothing pressed; a cell sets both axes; the centre cell is the tab stop", async () => {
    const { onanchor, cell, container } = mount(null);
    const pressed = container.querySelectorAll('[aria-pressed="true"]');
    expect(pressed).toHaveLength(0);
    expect(cell("Center").tabIndex).toBe(0);
    expect(cell("Top left").tabIndex).toBe(-1);
    expect(cell("Top left").title || cell("Top left").dataset.tip).toBe("Top left");
    await fireEvent.click(cell("Bottom right"));
    expect(onanchor).toHaveBeenLastCalledWith({ h: "right", v: "bottom" });
    await fireEvent.click(screen.getByRole("button", { name: "Center horizontally" }));
    expect(onanchor).toHaveBeenLastCalledWith({ h: "center" });
    await fireEvent.click(screen.getByRole("button", { name: "Center vertically" }));
    expect(onanchor).toHaveBeenLastCalledWith({ v: "middle" });
    await fireEvent.click(screen.getByRole("switch", { name: "Respect safe area" }));
    expect(onanchor).toHaveBeenLastCalledWith({ safe: true });
  });

  it("pressed states: the anchored point, centre buttons, a row for a single axis", () => {
    const { cell, unmount } = mount(a("center", "middle", true));
    expect(cell("Center").getAttribute("aria-pressed")).toBe("true");
    expect(cell("Top left").getAttribute("aria-pressed")).toBe("false");
    expect(screen.getByRole("button", { name: "Center horizontally" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByRole("button", { name: "Center vertically" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByRole("switch", { name: "Respect safe area" }).getAttribute("aria-checked")).toBe("true");
    unmount();
    const m = mount(a(null, "bottom"));
    const marked = ["Bottom left", "Bottom center", "Bottom right"];
    for (const n of marked) expect(m.cell(n).classList.contains("axis")).toBe(true);
    expect(m.cell("Center").classList.contains("axis")).toBe(false);
    expect(screen.getByRole("button", { name: "Center vertically" }).getAttribute("aria-pressed")).toBe("false");
  });

  it("pressing what is set lets it go", async () => {
    const { onanchor, cell } = mount(a("left", "top"));
    await fireEvent.click(cell("Top left"));
    expect(onanchor).toHaveBeenLastCalledWith({ h: null, v: null });
    cleanup();
    const m = mount(a("center", "top"));
    await fireEvent.click(screen.getByRole("button", { name: "Center horizontally" }));
    expect(m.onanchor).toHaveBeenLastCalledWith({ h: null });
  });

  it("mixed: nothing pressed", () => {
    const { container } = mount("mixed");
    expect(container.querySelectorAll('[aria-pressed="true"]')).toHaveLength(0);
  });

  it("arrow keys move within the grid (one tab stop), Enter chooses", async () => {
    const { cell, onanchor } = mount(a("left", "top"));
    expect(cell("Top left").tabIndex).toBe(0);
    cell("Top left").focus();
    await fireEvent.keyDown(cell("Top left"), { key: "ArrowRight" });
    expect(document.activeElement).toBe(cell("Top center"));
    expect(cell("Top center").tabIndex).toBe(0);
    expect(cell("Top left").tabIndex).toBe(-1);
    await fireEvent.keyDown(cell("Top center"), { key: "ArrowDown" });
    expect(document.activeElement).toBe(cell("Center"));
    await fireEvent.keyDown(cell("Center"), { key: "ArrowDown" });
    await fireEvent.keyDown(cell("Bottom center"), { key: "ArrowDown" });
    expect(document.activeElement).toBe(cell("Bottom center"));
    await fireEvent.keyDown(cell("Bottom center"), { key: "End" });
    expect(document.activeElement).toBe(cell("Bottom right"));
    await fireEvent.keyDown(cell("Bottom right"), { key: "Home" });
    expect(document.activeElement).toBe(cell("Top left"));
    expect(onanchor).not.toHaveBeenCalled();
    // Native buttons: Enter / Space click.
    await fireEvent.click(document.activeElement as HTMLElement);
    expect(onanchor).toHaveBeenCalledWith({ h: null, v: null });
  });
});

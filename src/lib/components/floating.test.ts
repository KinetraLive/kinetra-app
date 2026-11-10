import { afterEach, describe, expect, it, vi } from "vitest";
import { GAP, MARGIN, floating, place } from "./floating";

const VIEW = { width: 1280, height: 800 };
const box = (left: number, top: number, w: number, h: number) => ({ left, top, right: left + w, bottom: top + h });

describe("place", () => {
  it("opens below the anchor, left edges aligned", () => {
    expect(place(box(270, 130, 86, 28), { width: 256, height: 340 }, VIEW, "start")).toEqual({
      left: 270,
      top: 130 + 28 + GAP,
      maxHeight: null,
    });
  });

  it("aligns right edges with align end", () => {
    expect(place(box(270, 130, 86, 28), { width: 256, height: 100 }, VIEW, "end").left).toBe(270 + 86 - 256);
  });

  it("stays inside the window on both sides", () => {
    // Would hang off the right edge: pushed left.
    expect(place(box(1200, 100, 40, 28), { width: 256, height: 100 }, VIEW, "start").left).toBe(VIEW.width - MARGIN - 256);
    // Would hang off the left edge: pushed right.
    expect(place(box(20, 100, 40, 28), { width: 256, height: 100 }, VIEW, "end").left).toBe(MARGIN);
  });

  it("flips above when there is more room there, ending at the anchor", () => {
    const p = place(box(1000, 700, 28, 28), { width: 288, height: 220 }, VIEW, "end");
    expect(p.top + 220).toBe(700 - GAP);
    expect(p.maxHeight).toBeNull();
  });

  it("caps the height to the room it has (short window)", () => {
    const short = { width: 1100, height: 420 };
    const p = place(box(270, 130, 86, 28), { width: 256, height: 352 }, short, "end");
    // More room below (420 - 8 - 162 = 250) than above (130 - 4 - 8 = 118): stays below, capped.
    expect(p.top).toBe(130 + 28 + GAP);
    expect(p.maxHeight).toBe(420 - MARGIN - (158 + GAP));
    expect(p.top + (p.maxHeight ?? 0)).toBeLessThanOrEqual(short.height - MARGIN);
  });

  it("keeps a fitting panel below even when there is more room above", () => {
    const p = place(box(100, 600, 28, 28), { width: 200, height: 100 }, VIEW);
    expect(p.top).toBe(628 + GAP);
  });
});

describe("floating", () => {
  afterEach(() => document.body.replaceChildren());

  function mount() {
    const anchor = document.createElement("button");
    anchor.getBoundingClientRect = () => ({ ...box(300, 100, 80, 28), x: 300, y: 100, width: 80, height: 28, toJSON() {} });
    const panel = document.createElement("div");
    Object.defineProperty(panel, "offsetWidth", { value: 256 });
    Object.defineProperty(panel, "offsetHeight", { value: 200 });
    document.body.append(anchor, panel);
    return { anchor, panel };
  }

  it("shows the panel as a manual popover next to its anchor", () => {
    const { anchor, panel } = mount();
    let shown = 0;
    panel.showPopover = () => void shown++;
    const stop = floating(() => anchor, { align: "end" })(panel);
    expect(panel.getAttribute("popover")).toBe("manual");
    expect(shown).toBe(1);
    expect(panel.style.left).toBe(`${380 - 256}px`);
    expect(panel.style.top).toBe(`${128 + GAP}px`);
    stop();
  });

  it("still places the panel where popovers don't exist", () => {
    const { anchor, panel } = mount();
    Object.defineProperty(panel, "showPopover", { value: undefined });
    const stop = floating(() => anchor)(panel);
    // No popover attribute (it would hide the panel for good), still placed.
    expect(panel.hasAttribute("popover")).toBe(false);
    expect(panel.style.left).toBe("300px");
    stop();
  });

  it("removes its listeners when the panel goes", () => {
    const { anchor, panel } = mount();
    panel.showPopover = () => {};
    const add = vi.spyOn(window, "addEventListener");
    const remove = vi.spyOn(window, "removeEventListener");
    try {
      floating(() => anchor)(panel)();
      expect(add.mock.calls.map((c) => c[0]).sort()).toEqual(["resize", "scroll"]);
      expect(remove.mock.calls.map((c) => c[0]).sort()).toEqual(["resize", "scroll"]);
    } finally {
      add.mockRestore();
      remove.mockRestore();
    }
  });
});

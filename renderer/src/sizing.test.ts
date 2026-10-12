import { afterEach, describe, expect, it, vi } from "vitest";
import { App } from "./app";
import { Layer, scr } from "./layer";
import type { ScreenInfo, WireLayer } from "./protocol";
import { Stage } from "./stage";

const rows = (...r: Record<string, unknown>[]) => JSON.stringify(r);

function def(over: Partial<WireLayer> = {}): WireLayer {
  return {
    id: "a",
    z: 1,
    templateRev: 1,
    html: '<div class="g"><ul><li data-each="items">{{.name}}</li></ul></div>',
    css: ".g{position:absolute;inset:0}",
    fields: { items: rows({ name: "A" }, { name: "B" }) },
    fieldTypes: { items: "list" },
    state: "in",
    sizing: "fit",
    design: { w: 640, h: 360 },
    rect: { x: 100, y: 50, w: 320, h: 180 },
    ...over,
  };
}

/** The inner box's scale and offset, parsed from the transform the renderer set. */
function box(l: Layer) {
  const st = l.inner.style;
  const m = /translate\(([-\d.e]+)px, ?([-\d.e]+)px\) scale\(([-\d.e]+)\)/.exec(st.transform);
  return m ? { x: +m[1], y: +m[2], s: +m[3], w: st.width, h: st.height } : null;
}

/** Makes every element report the given computed animation duration (jsdom computes none). */
function fakeDuration(dur: string) {
  const real = window.getComputedStyle;
  return vi.spyOn(window, "getComputedStyle").mockImplementation((el: Element) => {
    const cs = real(el);
    return new Proxy(cs, {
      get: (t, p) => (p === "animationDuration" ? dur : p === "animationDelay" ? "0s" : Reflect.get(t, p, t)),
    });
  });
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
  document.body.replaceChildren();
  scr.w = scr.h = 0;
});

describe("content sizing", () => {
  it("fit: the design box scaled to the rect, the template inside it", () => {
    const l = new Layer(def(), document.createElement("div"));
    expect(box(l)).toEqual({ x: 0, y: 0, s: 0.5, w: "640px", h: "360px" });
    expect(l.inner.style.position).toBe("absolute");
    expect(l.inner.style.transformOrigin).toMatch(/^0(px)? 0(px)?$/);
    // The template's root lays out in the inner box (its containing block).
    expect(l.root!.parentNode).toBe(l.inner);
    expect(l.inner.parentNode).toBe(l.shadow);
  });

  it("fit letterboxes: uniform scale, centered on the other axis", () => {
    const tall = new Layer(def({ rect: { x: 0, y: 0, w: 640, h: 640 } }), document.createElement("div"));
    expect(box(tall)).toMatchObject({ x: 0, y: 140, s: 1 });
    const wide = new Layer(def({ rect: { x: 0, y: 0, w: 1920, h: 180 } }), document.createElement("div"));
    expect(box(wide)).toMatchObject({ x: 800, y: 0, s: 0.5 });
  });

  it("fit without a design uses the screen's size; without a rect the host is the screen", () => {
    scr.w = 1920;
    scr.h = 1080;
    const l = new Layer(def({ design: undefined, rect: { x: 0, y: 0, w: 960, h: 1080 } }), document.createElement("div"));
    expect(box(l)).toEqual({ x: 0, y: 270, s: 0.5, w: "1920px", h: "1080px" });
    const full = new Layer(def({ rect: undefined }), document.createElement("div"));
    expect(box(full)).toEqual({ x: 0, y: 0, s: 3, w: "640px", h: "360px" });
  });

  it("reflow (or absent): the content fills the host, as before", () => {
    for (const sizing of ["reflow", undefined] as const) {
      const l = new Layer(def({ sizing }), document.createElement("div"));
      expect(l.inner.style.display).toBe("contents");
      expect(l.inner.style.transform).toBe("");
    }
  });

  it("a rect message re-fits in place: no recompile, no replay", () => {
    const l = new Layer(def(), document.createElement("div"));
    const root = l.root!;
    const set = vi.spyOn(root, "setAttribute");
    l.setRect({ x: 0, y: 0, w: 1280, h: 720 });
    expect(box(l)).toMatchObject({ x: 0, y: 0, s: 2 });
    l.setRect({ x: 0, y: 0, w: 1280, h: 360 });
    expect(box(l)).toMatchObject({ x: 320, y: 0, s: 1 });
    expect(l.root).toBe(root);
    expect(set).not.toHaveBeenCalled();
  });

  it("a sizing change on a same-rev layer applies in place, on air, without a replay", () => {
    const stage = document.createElement("div");
    document.body.append(stage);
    const l = new Layer(def({ sizing: undefined }), stage);
    const root = l.root!;
    const set = vi.spyOn(root, "setAttribute");
    expect(l.inner.style.display).toBe("contents");
    l.apply(def());
    expect(box(l)).toMatchObject({ s: 0.5 });
    expect(l.root).toBe(root);
    expect(root.getAttribute("data-state")).toBe("in");
    l.apply(def({ sizing: undefined }));
    expect(l.inner.style.display).toBe("contents");
    // A new design size, same rev: re-fitted in place too.
    l.apply(def({ design: { w: 320, h: 90 } }));
    expect(box(l)).toEqual({ x: 0, y: 45, s: 1, w: "320px", h: "90px" });
    expect(l.root).toBe(root);
    expect(set).not.toHaveBeenCalled();
    expect(l.state).toBe("in");
  });

  it("an unchanged sizing writes no style", () => {
    const l = new Layer(def(), document.createElement("div"));
    const before = l.inner.style.cssText;
    const spy = vi.spyOn(l.inner.style, "cssText", "set");
    l.apply(def({ fields: { items: rows({ name: "C" }) } }));
    l.setRect(def().rect);
    expect(spy).not.toHaveBeenCalled();
    expect(l.inner.style.cssText).toBe(before);
  });

  it("snapshots carry the screen size; a new screen size re-fits a fit layer without a rect", () => {
    const stage = new Stage(document.body);
    const app = new App(stage);
    const screen = (width: number, height: number): ScreenInfo => ({ id: "s", name: "S", width, height, background: "transparent" });
    const l = def({ design: undefined, rect: undefined });
    app.handle({ t: "snapshot", screen: screen(1920, 1080), layers: [l] });
    const layer = app.layers.get("a")!;
    expect(box(layer)).toEqual({ x: 0, y: 0, s: 1, w: "1920px", h: "1080px" });
    app.handle({ t: "snapshot", screen: screen(1280, 720), layers: [l] });
    expect(app.layers.get("a")).toBe(layer);
    expect(box(layer)).toEqual({ x: 0, y: 0, s: 1, w: "1280px", h: "720px" });
  });

  it("swap hosts get the sizing too", () => {
    const stage = document.createElement("div");
    document.body.append(stage);
    const l = new Layer(def(), stage);
    l.apply(def({ templateRev: 2, html: '<p class="g">{{x}}</p>', fieldTypes: {}, fields: {} }), "crossfade");
    expect(box(l)).toMatchObject({ s: 0.5 });
    expect(l.root!.parentNode).toBe(l.inner);
  });
});

describe("list counts", () => {
  const ul = (l: Layer) => l.shadow.querySelector("ul")!;

  it("data-count and --n on the rows' parent follow the rows on screen", () => {
    const l = new Layer(def({ state: "out" }), document.createElement("div"));
    expect(ul(l).getAttribute("data-count")).toBe("2");
    expect(ul(l).style.getPropertyValue("--n")).toBe("2");
    l.patch({ items: rows({ name: "A" }, { name: "B" }, { name: "C" }) });
    expect(ul(l).getAttribute("data-count")).toBe("3");
    expect(ul(l).style.getPropertyValue("--n")).toBe("3");
    l.patch({ items: rows() });
    expect(ul(l).getAttribute("data-count")).toBe("0");
    expect(ul(l).hasAttribute("data-empty")).toBe(true);
  });

  it("rows animating out are not counted; unchanged counts are not written", () => {
    fakeDuration("0.5s");
    vi.useFakeTimers();
    const l = new Layer(def(), document.createElement("div"));
    const p = ul(l);
    l.patch({ items: rows({ name: "A" }) });
    expect(p.querySelectorAll("li").length).toBe(2);
    expect(p.getAttribute("data-count")).toBe("1");
    const set = vi.spyOn(p, "setAttribute");
    l.patch({ items: rows({ name: "Z" }) });
    expect(set).not.toHaveBeenCalled();
  });
});

describe("layer opacity", () => {
  it("goes on the inner box (fit and reflow), never on the host", () => {
    const fitL = new Layer(def({ opacity: 50 }), document.createElement("div"));
    expect(fitL.inner.style.opacity).toBe("0.5");
    expect(box(fitL)).toMatchObject({ s: 0.5 });
    expect(fitL.host.style.opacity).toBe("");
    const flow = new Layer(def({ sizing: undefined, opacity: 25 }), document.createElement("div"));
    expect(flow.inner.style.opacity).toBe("0.25");
    expect(flow.inner.style.position).toBe("absolute");
    expect(flow.inner.style.display).not.toBe("contents");
    expect(flow.host.style.opacity).toBe("");
    const zero = new Layer(def({ opacity: 0 }), document.createElement("div"));
    expect(zero.inner.style.opacity).toBe("0");
  });

  it("absent = 100: reflow stays display:contents", () => {
    const l = new Layer(def({ sizing: undefined }), document.createElement("div"));
    expect(l.inner.style.opacity).toBe("");
    expect(l.inner.style.display).toBe("contents");
  });

  it("a change on a same-rev layer applies in place, on air, without a replay", () => {
    const stage = document.createElement("div");
    document.body.append(stage);
    const l = new Layer(def({ sizing: undefined }), stage);
    const root = l.root!;
    const set = vi.spyOn(root, "setAttribute");
    l.apply(def({ sizing: undefined, opacity: 40 }));
    expect(l.inner.style.opacity).toBe("0.4");
    l.apply(def({ sizing: undefined, opacity: 80 }));
    expect(l.inner.style.opacity).toBe("0.8");
    l.apply(def({ sizing: undefined }));
    expect(l.inner.style.opacity).toBe("");
    expect(l.inner.style.display).toBe("contents");
    expect(l.root).toBe(root);
    expect(set).not.toHaveBeenCalled();
    expect(l.state).toBe("in");
  });

  it("a crossfade animates the hosts while the inner box keeps the layer opacity", () => {
    const stage = document.createElement("div");
    document.body.append(stage);
    const l = new Layer(def({ opacity: 60 }), stage);
    l.apply(def({ opacity: 60, templateRev: 2, html: '<p class="g">{{x}}</p>', fieldTypes: {}, fields: {} }), "crossfade");
    expect(l.inner.style.opacity).toBe("0.6");
    expect(l.host.style.opacity).toBe("");
  });
});

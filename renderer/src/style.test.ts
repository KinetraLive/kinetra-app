// Style library markup: data-empty, data-fit and the font preload before play-in.
import { env } from "node:process";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./app";
import { FONT_CAP_MS, faces, resetFonts } from "./fonts";
import { Layer, compile, fit } from "./layer";
import type { FieldType, Fields, ScreenInfo, WireLayer } from "./protocol";
import { Stage } from "./stage";

const rows = (...r: Record<string, unknown>[]) => JSON.stringify(r);

function render(html: string, fields: Fields, types: Record<string, FieldType> = {}) {
  const c = compile(html, types, (k) => fields[k] ?? "");
  const div = document.createElement("div");
  div.append(c.frag);
  return { ...c, div };
}

function def(over: Partial<WireLayer> = {}): WireLayer {
  return {
    id: "l1",
    z: 1,
    templateRev: 1,
    html: '<div class="lt"><h1 class="name" data-fit>{{name}}</h1><p class="title">{{title}}</p></div>',
    css: "",
    fields: { name: "Ada", title: "Engineer" },
    fieldTypes: { name: "text", title: "text" },
    state: "out",
    ...over,
  };
}

const empty = (el: Element | null | undefined) => el!.hasAttribute("data-empty");

/**
 * jsdom has no layout. Fake it for `[data-fit]` elements: text is 0.5 em per character, the box is
 * 400 px wide, line height 1.2 em, CSS font size 40 px. `data-wrap` elements wrap (auto height),
 * others overflow to the right. Counts layout reads and records the root's data-state at the first.
 */
const BASE = 40;
const BOX = 400;
const probe = { reads: 0, stateAtFirstRead: undefined as string | null | undefined };
function fakeLayout() {
  const P = HTMLElement.prototype;
  const fs = (el: HTMLElement) => parseFloat(el.style.fontSize) || BASE;
  const w = (el: HTMLElement) => (el.textContent || "").length * fs(el) * 0.5;
  const lines = (el: HTMLElement) => Math.max(1, Math.ceil(w(el) / BOX));
  const on = (el: HTMLElement) => el.hasAttribute("data-fit");
  const metrics: Record<string, (el: HTMLElement) => number> = {
    scrollWidth: (el) => {
      if (probe.reads++ === 0) probe.stateAtFirstRead = (el.getRootNode() as ShadowRoot).firstElementChild?.nextElementSibling?.getAttribute("data-state");
      return el.hasAttribute("data-wrap") ? BOX : Math.max(BOX, w(el));
    },
    clientWidth: () => BOX,
    scrollHeight: (el) => (el.hasAttribute("data-wrap") ? lines(el) : 1) * fs(el) * 1.2,
    clientHeight: (el) => (el.hasAttribute("data-wrap") ? lines(el) : 1) * fs(el) * 1.2,
  };
  for (const [k, f] of Object.entries(metrics))
    Object.defineProperty(P, k, { configurable: true, get(this: HTMLElement) { return on(this) ? f(this) : 0; } });
  const real = window.getComputedStyle;
  vi.spyOn(window, "getComputedStyle").mockImplementation((el: Element) =>
    on(el as HTMLElement) ? ({ fontSize: fs(el as HTMLElement) + "px", lineHeight: "normal" } as CSSStyleDeclaration) : real(el),
  );
  return () => {
    for (const k of Object.keys(metrics)) delete (P as unknown as Record<string, unknown>)[k];
  };
}

const size = (el: Element | null) => (el as HTMLElement).style.fontSize;
const px = (el: Element | null) => parseFloat(size(el));

let unfake: (() => void) | undefined;
beforeEach(() => {
  probe.reads = 0;
  probe.stateAtFirstRead = undefined;
  resetFonts();
});
afterEach(() => {
  unfake?.();
  unfake = undefined;
  vi.restoreAllMocks();
  vi.useRealTimers();
  document.body.replaceChildren();
  document.head.querySelectorAll("link[data-font]").forEach((n) => n.remove());
  delete (document as unknown as { fonts?: unknown }).fonts;
});

describe("data-empty", () => {
  it("marks text and image elements whose bound value is empty, and clears it when filled", () => {
    const l = new Layer(
      def({
        html: '<div class="lt"><span class="t">{{title}}</span><img src="{{photo}}"><b class="{{cls}}">x</b></div>',
        fields: { title: "", photo: "", cls: "" },
        fieldTypes: { title: "text", photo: "image", cls: "text" },
      }),
      document.body,
    );
    const t = l.shadow.querySelector(".t");
    const img = l.shadow.querySelector("img");
    expect(empty(t)).toBe(true);
    expect(empty(img)).toBe(true);
    // A binding in a non-URL attribute (class, style, title) does not make an element "empty".
    expect(empty(l.shadow.querySelector("b"))).toBe(false);
    expect(empty(l.root)).toBe(false);

    l.patch({ title: "Engineer", photo: "/assets/a.png" });
    expect(empty(t)).toBe(false);
    expect(empty(img)).toBe(false);
    l.patch({ title: "   " }); // whitespace only counts as empty
    expect(empty(t)).toBe(true);
    l.patch({ photo: "" });
    expect(empty(img)).toBe(true);
  });

  it("an element showing several keys is empty only when all of them are", () => {
    const { div, binds } = render("<p>{{first}} {{last}}</p>", { first: "", last: "" });
    const p = div.querySelector("p")!;
    expect(empty(p)).toBe(true);
    expect(binds.get("first")?.length).toBeGreaterThan(0);
    const c = render("<p>{{first}} {{last}}</p>", { first: "Ada", last: "" });
    expect(empty(c.div.querySelector("p"))).toBe(false);
  });

  it("marks cells inside list rows, follows row updates, and marks the row container when no rows", () => {
    const html = '<ul class="list"><li data-each="items"><b class="n">{{.name}}</b><i class="s">{{.sub}}</i></li></ul>';
    const l = new Layer(
      def({ html, fields: { items: rows({ name: "Ada", sub: "" }, { name: "", sub: "x" }) }, fieldTypes: { items: "list" } }),
      document.body,
    );
    const cells = (sel: string) => Array.from(l.shadow.querySelectorAll(sel)).map(empty);
    expect(cells(".n")).toEqual([false, true]);
    expect(cells(".s")).toEqual([true, false]);
    expect(empty(l.shadow.querySelector("ul"))).toBe(false);

    l.patch({ items: rows({ name: "Ada", sub: "Chair" }, { name: "Grace", sub: "" }) });
    expect(cells(".n")).toEqual([false, false]);
    expect(cells(".s")).toEqual([false, true]);

    l.patch({ items: "[]" });
    expect(empty(l.shadow.querySelector("ul"))).toBe(true);
    l.patch({ items: rows({ name: "Ada" }) });
    expect(empty(l.shadow.querySelector("ul"))).toBe(false);
  });

  it("an html field and a clock are judged by value", () => {
    const { div } = render('<div class="h">{{bio}}</div><time>{{now}}</time>', { bio: "", now: "HH:mm" }, { bio: "html", now: "clock" });
    expect(empty(div.querySelector(".h"))).toBe(true);
    expect(empty(div.querySelector("time"))).toBe(false);
  });
});

describe("data-fit", () => {
  it("leaves text that fits at the CSS size", () => {
    unfake = fakeLayout();
    const l = new Layer(def({ state: "in", fields: { name: "Ada Lovelace" } }), document.body);
    expect(size(l.shadow.querySelector(".name"))).toBe("");
  });

  it("shrinks text too wide for one line by proportion: two layouts", () => {
    unfake = fakeLayout();
    // 25 chars × 20 px = 500 px in a 400 px box: fits from 0.8 × 40 = 32 px down.
    const l = new Layer(def({ state: "in", fields: { name: "A".repeat(25) } }), document.body);
    const v = px(l.shadow.querySelector(".name"));
    expect(v).toBeLessThanOrEqual(32);
    expect(v).toBeGreaterThan(31);
    expect(probe.reads).toBe(2); // the CSS size, then clientWidth / scrollWidth
  });

  it("stops at data-fit-min (default 0.6)", () => {
    unfake = fakeLayout();
    const l = new Layer(def({ state: "in", fields: { name: "A".repeat(100) } }), document.body);
    expect(px(l.shadow.querySelector(".name"))).toBe(BASE * 0.6);
    const m = new Layer(
      def({ id: "l2", state: "in", html: '<div><h1 data-fit data-fit-min="0.5">{{name}}</h1></div>', fields: { name: "A".repeat(100) } }),
      document.body,
    );
    expect(px(m.shadow.querySelector("h1"))).toBe(BASE * 0.5);
  });

  it("data-fit-lines lets wrapping text use that many lines before shrinking", () => {
    unfake = fakeLayout();
    // 30 chars × 20 px = 600 px: two lines at full size.
    const html = (lines: string) => `<div><h1 data-fit data-wrap ${lines}>{{name}}</h1></div>`;
    const two = new Layer(def({ state: "in", html: html('data-fit-lines="2"'), fields: { name: "A".repeat(30) } }), document.body);
    expect(size(two.shadow.querySelector("h1"))).toBe("");
    const one = new Layer(def({ id: "l2", state: "in", html: html(""), fields: { name: "A".repeat(30) } }), document.body);
    const v = px(one.shadow.querySelector("h1"));
    expect(v).toBeLessThanOrEqual((BASE * 2) / 3); // one line: 30 × 0.5 × fs ≤ 400
    expect(v).toBeGreaterThan(BASE * 0.6);
  });

  it("fits before the root gets data-state=in, in the same call", () => {
    unfake = fakeLayout();
    const l = new Layer(def({ fields: { name: "A".repeat(25) } }), document.body);
    expect(probe.reads).toBe(0); // off air: nothing measured
    l.setState("in");
    expect(probe.reads).toBeGreaterThan(0);
    expect(probe.stateAtFirstRead).not.toBe("in");
    expect(l.root!.getAttribute("data-state")).toBe("in");
    expect(px(l.shadow.querySelector(".name"))).toBeLessThanOrEqual(32);
  });

  it("a field patch on air fits in the same call, only the changed element, in at most 7 layouts", () => {
    unfake = fakeLayout();
    const l = new Layer(def({ state: "in" }), document.body);
    const name = l.shadow.querySelector(".name");
    expect(size(name)).toBe("");
    probe.reads = 0;
    l.patch({ title: "Something else entirely" }); // not a data-fit element
    expect(probe.reads).toBe(0);
    const t0 = performance.now();
    l.patch({ name: "N".repeat(100) });
    const ms = performance.now() - t0;
    expect(px(name)).toBe(BASE * 0.6);
    expect(probe.reads).toBeLessThanOrEqual(3); // too wide even at the minimum: no search
    // 4 ms here; shared CI runners are several times slower and noisier, so CI only catches a
    // real slowdown.
    expect(ms).toBeLessThan(env.CI ? 20 : 4);
    l.patch({ name: "Ada" });
    expect(size(name)).toBe(""); // back to the CSS size
  });

  it("data-fit on a wrapper fits when bound text inside it changes; rows fit too", () => {
    unfake = fakeLayout();
    const l = new Layer(
      def({
        state: "in",
        html: '<div><h1 data-fit><span>{{name}}</span></h1><ul><li data-each="items"><b data-fit>{{.n}}</b></li></ul></div>',
        fields: { name: "Ada", items: rows({ n: "x" }) },
        fieldTypes: { name: "text", items: "list" },
      }),
      document.body,
    );
    l.patch({ name: "A".repeat(100), items: rows({ n: "B".repeat(100) }, { n: "ok" }) });
    expect(px(l.shadow.querySelector("h1"))).toBe(BASE * 0.6);
    const bs = l.shadow.querySelectorAll("b");
    expect(px(bs[0])).toBe(BASE * 0.6);
    expect(size(bs[1])).toBe("");
  });

  it("a detached element is left at its CSS size", () => {
    unfake = fakeLayout();
    const h = document.createElement("h1");
    h.setAttribute("data-fit", "");
    h.textContent = "A".repeat(100);
    h.style.fontSize = "12px";
    fit(h);
    expect(h.style.fontSize).toBe("");
    expect(probe.reads).toBe(0);
  });

  it("a rect change fits again", () => {
    unfake = fakeLayout();
    const l = new Layer(def({ state: "in", fields: { name: "A".repeat(25) } }), document.body);
    probe.reads = 0;
    l.setRect({ x: 0, y: 0, w: 800, h: 200 });
    expect(probe.reads).toBeGreaterThan(0);
  });
});

describe("--auto-out-ms", () => {
  const prop = (l: Layer) => l.host.style.getPropertyValue("--auto-out-ms");

  it("is set on the host from the template's autoOutMs, absent otherwise, and follows same-rev updates", () => {
    const l = new Layer(def({ autoOutMs: 5000 }), document.body);
    expect(prop(l)).toBe("5000ms");
    const root = l.root;
    l.apply(def({ autoOutMs: 8000 })); // same rev: in place
    expect(prop(l)).toBe("8000ms");
    expect(l.root).toBe(root); // no recompile
    l.apply(def({}));
    expect(prop(l)).toBe("");
    expect(prop(new Layer(def({ id: "l2" }), document.body))).toBe("");
  });

  it("survives a recompile (new rev) and a variable replace", () => {
    const l = new Layer(def({ autoOutMs: 5000, vars: { accent: "#f00" } }), document.body);
    l.apply(def({ templateRev: 2, autoOutMs: 3000, vars: {} }));
    expect(prop(l)).toBe("3000ms");
    l.setVars({ accent: "#0f0" }, true);
    expect(prop(l)).toBe("3000ms");
  });
});

describe("data-fit is strict (no ellipsis on text that fits)", () => {
  /**
   * One element with sub-pixel layout: its content is `at40` px wide at 40 px (proportional to
   * the font size), a 400 px content box (+ 8 px padding each side, so clientWidth 416) shown at
   * half scale (a transform: client rects are halved). scrollWidth rounds, like a browser.
   */
  function subpixel(at40: number) {
    const h = document.createElement("h1");
    h.setAttribute("data-fit", "");
    h.textContent = "Ada Lovelace";
    document.body.append(h);
    const fs = () => parseFloat(h.style.fontSize) || 40;
    const content = () => (at40 * fs()) / 40;
    const own = (k: string, get: () => number) => Object.defineProperty(h, k, { configurable: true, get });
    own("clientWidth", () => 416);
    own("scrollWidth", () => Math.max(416, Math.round(content() + 16)));
    own("scrollHeight", () => fs() * 1.2);
    h.getBoundingClientRect = () => ({ width: 208 }) as DOMRect;
    // jsdom has no Range layout: one per test (the last subpixel() call wins).
    Object.defineProperty(Range.prototype, "getBoundingClientRect", { configurable: true, value: () => ({ width: content() / 2 }) });
    unfake = () => delete (Range.prototype as unknown as Record<string, unknown>).getBoundingClientRect;
    const real = window.getComputedStyle;
    vi.spyOn(window, "getComputedStyle").mockImplementation((el: Element) =>
      el === h
        ? ({
            fontSize: fs() + "px",
            lineHeight: "normal",
            width: "416px",
            boxSizing: "border-box",
            paddingLeft: "8px",
            paddingRight: "8px",
            borderLeftWidth: "0px",
            borderRightWidth: "0px",
          } as CSSStyleDeclaration)
        : real(el),
    );
    fit(h);
    return { size: h.style.fontSize, content: content() };
  }

  it("shrinks content 1 px too wide (scrollWidth one over clientWidth)", () => {
    const r = subpixel(400.6); // scrollWidth 417, clientWidth 416
    expect(r.size).not.toBe("");
    expect(r.content).toBeLessThanOrEqual(400);
  });

  it("shrinks content a fraction of a pixel too wide (scrollWidth rounds to clientWidth)", () => {
    const r = subpixel(400.3); // scrollWidth 416 = clientWidth, yet it overhangs
    expect(r.size).not.toBe("");
    expect(r.content).toBeLessThanOrEqual(400);
    expect(r.content).toBeGreaterThan(395); // a hair smaller, not a big jump
  });

  it("leaves content that fits (to the sub-pixel) at the CSS size", () => {
    expect(subpixel(399.7).size).toBe("");
    expect(subpixel(400).size).toBe("");
  });
});

/** A controllable document.fonts. */
function fakeFonts() {
  const pending: { d: string; resolve: () => void }[] = [];
  const load = vi.fn((d: string) => new Promise<void>((resolve) => pending.push({ d, resolve })));
  Object.defineProperty(document, "fonts", { configurable: true, value: { load } });
  return { load, pending, done: () => pending.splice(0).forEach((p) => p.resolve()) };
}

/** Fires `load` on every font stylesheet link (jsdom never loads them). */
const sheetsLoaded = () => document.head.querySelectorAll("link[data-font]").forEach((l) => l.dispatchEvent(new Event("load")));

const FRAME_CSS = '.lt{font-family:"Space Grotesk",sans-serif;font-weight:600}.t{font:500 20px/1.2 "IBM Plex Mono"}';

describe("font preload", () => {
  it("names every curated family × the weights the CSS uses (plus 400), variables included", () => {
    const f = faces(FRAME_CSS, ['"Barlow", sans-serif']).map(([, d]) => d);
    expect(f.sort()).toEqual(
      [
        '400 16px "IBM Plex Mono"',
        '400 16px "Barlow"',
        '400 16px "Space Grotesk"',
        '500 16px "IBM Plex Mono"',
        '500 16px "Barlow"',
        '500 16px "Space Grotesk"',
        '600 16px "IBM Plex Mono"',
        '600 16px "Barlow"',
        '600 16px "Space Grotesk"',
      ].sort(),
    );
    expect(faces("h1{font-weight:bold;font-family:Oswald}", []).map(([, d]) => d)).toEqual(['400 16px "Oswald"', '700 16px "Oswald"']);
    expect(faces("h1{font-family:system-ui}", [])).toEqual([]);
  });

  it("adds the italic faces when the CSS sets an italic style (font-style or the font shorthand)", () => {
    const d = (css: string) => faces(css, []).map(([, x]) => x);
    expect(d('h1{font-family:"Playfair Display";font-weight:600}em{font-style:italic}').sort()).toEqual(
      [
        '400 16px "Playfair Display"',
        '600 16px "Playfair Display"',
        'italic 400 16px "Playfair Display"',
        'italic 600 16px "Playfair Display"',
      ].sort(),
    );
    expect(d('q{font:italic 400 20px/1.3 "Source Serif 4", serif}')).toEqual(['400 16px "Source Serif 4"', 'italic 400 16px "Source Serif 4"']);
    expect(d('h1{font-family:"Playfair Display";font-style:normal}')).toEqual(['400 16px "Playfair Display"']);
  });

  it("waits for the stylesheet and the fonts before playing in; then fits and plays in", async () => {
    const fonts = fakeFonts();
    unfake = fakeLayout();
    const l = new Layer(def({ css: FRAME_CSS, fields: { name: "A".repeat(25) } }), document.body);
    l.setState("in");
    expect(l.host.isConnected).toBe(false);
    expect(l.root!.getAttribute("data-state")).toBeNull();
    await Promise.resolve();
    expect(fonts.load).not.toHaveBeenCalled(); // the @font-face rules are not known yet
    sheetsLoaded();
    await vi.waitFor(() => expect(fonts.load).toHaveBeenCalledTimes(6)); // 2 families × 400/500/600
    expect(fonts.load.mock.calls.map(([d]) => d)).toContain('600 16px "Space Grotesk"');
    l.setState("in"); // a second take while waiting changes nothing
    expect(l.state).toBeNull();
    fonts.done();
    await vi.waitFor(() => expect(l.root!.getAttribute("data-state")).toBe("in"));
    expect(l.state).toBe("in");
    expect(probe.stateAtFirstRead).not.toBe("in");
    expect(px(l.shadow.querySelector(".name"))).toBeLessThanOrEqual(32);
  });

  it("fonts already loaded: plays in at once", async () => {
    const fonts = fakeFonts();
    const a = new Layer(def({ css: FRAME_CSS }), document.body); // preloads at build
    sheetsLoaded();
    await vi.waitFor(() => expect(fonts.load).toHaveBeenCalled());
    fonts.done();
    await new Promise((r) => setTimeout(r, 0));
    a.setState("in");
    expect(a.root!.getAttribute("data-state")).toBe("in");
    const b = new Layer(def({ id: "l2", css: FRAME_CSS, state: "in" }), document.body);
    expect(b.root!.getAttribute("data-state")).toBe("in");
  });

  it("plays in after FONT_CAP_MS when fonts never arrive; no timer left", async () => {
    vi.useFakeTimers();
    fakeFonts();
    const l = new Layer(def({ css: FRAME_CSS, state: "in" }), document.body);
    sheetsLoaded();
    await vi.advanceTimersByTimeAsync(FONT_CAP_MS - 1);
    expect(l.root!.getAttribute("data-state")).toBeNull();
    await vi.advanceTimersByTimeAsync(1);
    expect(l.root!.getAttribute("data-state")).toBe("in");
    expect(vi.getTimerCount()).toBe(0);
  });

  it("a hide, cut or new template while waiting cancels the play-in and its timer", async () => {
    vi.useFakeTimers();
    const fonts = fakeFonts();
    const app = new App(new Stage(document.body));
    const screen: ScreenInfo = { id: "s", name: "S", width: 1920, height: 1080, background: "transparent" };
    app.handle({ t: "snapshot", screen, layers: [def({ css: FRAME_CSS, state: "in" })] });
    const l = app.layers.get("l1")!;
    expect(vi.getTimerCount()).toBe(1);
    app.handle({ t: "state", layerId: "l1", state: "out" });
    expect(vi.getTimerCount()).toBe(0);
    sheetsLoaded();
    await vi.advanceTimersByTimeAsync(0);
    fonts.done();
    await vi.advanceTimersByTimeAsync(FONT_CAP_MS * 2);
    expect(l.host.isConnected).toBe(false);
    expect(l.root!.getAttribute("data-state")).toBeNull();

    app.handle({ t: "state", layerId: "l1", state: "in" }); // fonts known now: at once
    expect(l.root!.getAttribute("data-state")).toBe("in");

    resetFonts();
    app.handle({ t: "cut" });
    app.handle({ t: "state", layerId: "l1", state: "in" });
    expect(vi.getTimerCount()).toBe(1);
    app.handle({ t: "cut" });
    expect(vi.getTimerCount()).toBe(0);
  });

  it("a crossfade to a look with new fonts keeps the old look until they load", async () => {
    vi.useFakeTimers();
    const fonts = fakeFonts();
    const app = new App(new Stage(document.body));
    const screen: ScreenInfo = { id: "s", name: "S", width: 1920, height: 1080, background: "transparent" };
    app.handle({ t: "snapshot", screen, layers: [def({ state: "in" })] });
    const l = app.layers.get("l1")!;
    const old = l.host;
    app.handle({ t: "layer", layer: def({ templateRev: 2, css: FRAME_CSS, state: "in", swap: "crossfade" }) });
    expect(old.isConnected).toBe(true);
    expect(l.host.isConnected).toBe(false); // waiting for Space Grotesk
    sheetsLoaded();
    await vi.advanceTimersByTimeAsync(0);
    fonts.done();
    await vi.advanceTimersByTimeAsync(0);
    expect(l.host.isConnected).toBe(true);
    expect(l.root!.getAttribute("data-state")).toBe("in");
    await vi.advanceTimersByTimeAsync(1000);
    expect(old.isConnected).toBe(false);
    expect(vi.getTimerCount()).toBe(0);

    // A new rev while still waiting: the waiting look shows at once and crossfades on.
    resetFonts();
    app.handle({ t: "layer", layer: def({ templateRev: 3, css: FRAME_CSS, state: "in", swap: "crossfade" }) });
    const waiting = l.host;
    expect(waiting.isConnected).toBe(false);
    app.handle({ t: "layer", layer: def({ templateRev: 4, state: "in", swap: "crossfade" }) });
    expect(waiting.isConnected).toBe(true); // the old look of the fade to rev 4
    expect(l.host.isConnected).toBe(true);
    await vi.advanceTimersByTimeAsync(1000);
    expect(waiting.isConnected).toBe(false);
    expect(l.root!.getAttribute("data-state")).toBe("in");
    expect(vi.getTimerCount()).toBe(0);
  });

  it("no timers at rest after a show, patches, a rect change and a hide", async () => {
    vi.useFakeTimers();
    unfake = fakeLayout();
    const fonts = fakeFonts();
    const l = new Layer(def({ css: FRAME_CSS, state: "in" }), document.body);
    sheetsLoaded();
    await vi.advanceTimersByTimeAsync(0);
    fonts.done();
    await vi.advanceTimersByTimeAsync(0);
    expect(l.root!.getAttribute("data-state")).toBe("in");
    expect(vi.getTimerCount()).toBe(0);
    l.patch({ name: "N".repeat(100), title: "" });
    l.setRect({ x: 0, y: 0, w: 960, h: 200 });
    l.setVars({ accent: "#ff0000" });
    await vi.advanceTimersByTimeAsync(0);
    expect(vi.getTimerCount()).toBe(0);
    l.setState("out");
    await vi.advanceTimersByTimeAsync(3000);
    expect(l.host.isConnected).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });
});

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./app";
import { FADE_MS } from "./layer";
import type { ScreenInfo, WireLayer } from "./protocol";
import { Stage } from "./stage";

const screen: ScreenInfo = { id: "test", name: "Test", width: 1920, height: 1080, background: "transparent" };

function layer(over: Partial<WireLayer> = {}): WireLayer {
  return {
    id: "lt",
    z: 4,
    templateRev: 1,
    html: '<div class="v1"><span class="name">{{name}}</span></div>',
    css: "",
    fields: { name: "Ada" },
    fieldTypes: { name: "text" },
    state: "in",
    ...over,
  };
}

/** Variant n: a new rev and a new look. */
const variant = (n: number, over: Partial<WireLayer> = {}) =>
  layer({ templateRev: n, html: `<p class="v${n}">{{name}}</p>`, ...over });

function setup(layers = [layer()]) {
  const stage = new Stage(document.body);
  const app = new App(stage);
  app.handle({ t: "snapshot", screen, layers });
  return { app, stage, l: app.layers.get("lt")! };
}

/** Makes `root`'s out-animation pending until the returned function is called. */
function pendingOut(root: Element) {
  let finish!: () => void;
  const finished = new Promise<void>((r) => (finish = r));
  root.getAnimations = () => [{ finished } as unknown as Animation];
  return finish;
}

const hosts = (s: Stage) => Array.from(s.box.children) as HTMLElement[];
const look = (h: HTMLElement) => h.shadowRoot!.querySelector("[data-state]") ?? h.shadowRoot!.querySelector("div,p");

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

describe("variant swap", () => {
  it("crossfade keeps both hosts during the swap, then only the new one", () => {
    const { app, stage, l } = setup();
    const old = l.host;
    // Without a swap: recompiled in place on the same host.
    app.handle({ t: "layer", layer: variant(2) });
    expect(l.host).toBe(old);
    app.handle({ t: "layer", layer: variant(3, { swap: "crossfade" }) });
    const [a, b] = hosts(stage);
    expect(hosts(stage).length).toBe(2);
    expect(a).toBe(old);
    expect(b).toBe(l.host);
    expect(look(a)!.className).toBe("v2");
    expect(look(a)!.getAttribute("data-state")).toBe("in"); // the old look is not animated out
    expect(look(b)!.className).toBe("v3");
    expect(look(b)!.getAttribute("data-state")).toBe("in");
    expect(b.style.zIndex).toBe("4");
    vi.advanceTimersByTime(FADE_MS + 20);
    expect(hosts(stage)).toEqual([b]);
    expect(old.isConnected).toBe(false);
    expect(l.state).toBe("in");
    expect(vi.getTimerCount()).toBe(0); // nothing scheduled at rest
  });

  it("crossfade animates only host opacity and drops the fill when done", () => {
    const calls: { el: Element; k: unknown; cancel: () => void }[] = [];
    const proto = HTMLElement.prototype as unknown as { animate?: unknown };
    proto.animate = function (this: Element, k: unknown) {
      const c = { el: this, k, cancel: vi.fn() };
      calls.push(c);
      return c;
    };
    try {
      const { app, l } = setup();
      const old = l.host;
      app.handle({ t: "layer", layer: variant(2, { swap: "crossfade" }) });
      expect(calls.map((c) => [c.el, c.k])).toEqual([
        [l.host, { opacity: [0, 1] }],
        [old, { opacity: [1, 0] }],
      ]);
      vi.advanceTimersByTime(FADE_MS + 20);
      expect(calls[0].cancel).toHaveBeenCalled();
    } finally {
      delete proto.animate;
    }
  });

  it("the new look appears settled: its finite in-animations are finished, loops keep running", () => {
    const fin = vi.fn();
    const loop = vi.fn();
    const proto = Element.prototype as unknown as { getAnimations?: unknown };
    proto.getAnimations = () => [
      { effect: { getComputedTiming: () => ({ iterations: 1 }) }, finish: fin },
      { effect: { getComputedTiming: () => ({ iterations: Infinity }) }, finish: loop },
    ];
    try {
      const { app } = setup();
      app.handle({ t: "layer", layer: variant(2, { swap: "crossfade" }) });
      expect(fin).toHaveBeenCalledTimes(1);
      expect(loop).not.toHaveBeenCalled();
    } finally {
      delete proto.getAnimations;
    }
  });

  it("carries fields, vars and rect to the new host", () => {
    const rect = { x: 10, y: 20, w: 300, h: 80 };
    const { app, stage, l } = setup([layer({ rect, vars: { accent: "#ff0000" } })]);
    app.handle({ t: "fields", layerId: "lt", fields: { name: "Grace" } });
    app.handle({
      t: "layer",
      layer: variant(2, { rect, vars: { accent: "#00ff00", size: "2" }, fields: { name: "Grace" }, swap: "crossfade" }),
    });
    const h = l.host;
    expect(hosts(stage).length).toBe(2);
    expect([h.style.left, h.style.top, h.style.width, h.style.height]).toEqual(["10px", "20px", "300px", "80px"]);
    expect(h.style.getPropertyValue("--accent")).toBe("#00ff00");
    expect(h.style.getPropertyValue("--size")).toBe("2");
    expect(look(h)!.textContent).toBe("Grace");
    // Patches during the swap go to the new look.
    app.handle({ t: "fields", layerId: "lt", fields: { name: "Linus" } });
    app.handle({ t: "vars", layerId: "lt", vars: { accent: "#0000ff" } });
    expect(look(h)!.textContent).toBe("Linus");
    expect(h.style.getPropertyValue("--accent")).toBe("#0000ff");
  });

  it("crossfade ignores reduced motion: both looks overlap for FADE_MS", () => {
    vi.stubGlobal("matchMedia", (q: string) => ({ matches: q.includes("reduce") }));
    const { app, stage, l } = setup();
    const old = l.host;
    app.handle({ t: "layer", layer: variant(2, { swap: "crossfade" }) });
    expect(hosts(stage)).toEqual([old, l.host]);
    expect(look(l.host)!.className).toBe("v2");
    vi.advanceTimersByTime(FADE_MS + 30);
    expect(hosts(stage)).toEqual([l.host]);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("outIn plays the old look out, then the new one in", async () => {
    const { app, stage, l } = setup();
    const old = l.host;
    const oldRoot = l.root!;
    const finish = pendingOut(oldRoot);
    app.handle({ t: "layer", layer: variant(2, { swap: "outIn", fields: { name: "Grace" } }) });
    expect(hosts(stage)).toEqual([old]);
    expect(oldRoot.getAttribute("data-state")).toBe("out");
    expect(l.host.isConnected).toBe(false);
    finish();
    await vi.waitFor(() => expect(hosts(stage)).toEqual([l.host]));
    expect(look(l.host)!.className).toBe("v2");
    expect(look(l.host)!.getAttribute("data-state")).toBe("in");
    expect(look(l.host)!.textContent).toBe("Grace");
    expect(vi.getTimerCount()).toBe(0);
  });

  it("outIn falls back to the computed-duration timer", () => {
    const { app, stage, l } = setup();
    pendingOut(l.root!);
    app.handle({ t: "layer", layer: variant(2, { swap: "outIn" }) });
    expect(hosts(stage).length).toBe(1);
    vi.advanceTimersByTime(3000);
    expect(hosts(stage)).toEqual([l.host]);
    expect(look(l.host)!.getAttribute("data-state")).toBe("in");
  });

  it("no swap off air, on a take, or in a snapshot", () => {
    // Off air: recompiled, nothing shown.
    const off = setup([layer({ state: "out" })]);
    off.app.handle({ t: "layer", layer: variant(2, { state: "out", swap: "crossfade" }) });
    expect(hosts(off.stage).length).toBe(0);
    document.body.replaceChildren();

    // Going on air with a new rev and a swap flag: a plain take, one host.
    const take = setup([layer({ state: "out" })]);
    take.app.handle({ t: "layer", layer: variant(2, { swap: "crossfade" }) });
    expect(hosts(take.stage)).toEqual([take.l.host]);
    expect(vi.getTimerCount()).toBe(0);
    document.body.replaceChildren();

    // Snapshot (reconnect) with a new rev: recompiled in place on the same host.
    const { app, stage, l } = setup();
    const host = l.host;
    app.handle({ t: "snapshot", screen, layers: [variant(2, { swap: "crossfade" })] });
    expect(hosts(stage)).toEqual([host]);
    expect(l.host).toBe(host);
    expect(look(host)!.className).toBe("v2");
    // Same rev with a swap flag: patched in place.
    app.handle({ t: "layer", layer: variant(2, { swap: "outIn", fields: { name: "Grace" } }) });
    expect(hosts(stage)).toEqual([host]);
    expect(look(host)!.getAttribute("data-state")).toBe("in");
    expect(look(host)!.textContent).toBe("Grace");
  });

  it("rapid switches never leave orphan hosts; node count stays flat", () => {
    const { app, stage, l } = setup();
    const count = () => stage.box.querySelectorAll("*").length + l.shadow.querySelectorAll("*").length;
    app.handle({ t: "layer", layer: variant(2, { swap: "crossfade" }) });
    vi.advanceTimersByTime(FADE_MS + 20);
    const n = count();
    for (let i = 3; i < 23; i++) {
      if (i % 3 === 0) pendingOut(l.root!);
      app.handle({ t: "layer", layer: variant(i, { swap: i % 3 ? "crossfade" : "outIn" }) });
      expect(hosts(stage).length).toBeLessThanOrEqual(2);
    }
    vi.advanceTimersByTime(3000);
    expect(hosts(stage)).toEqual([l.host]);
    expect(look(l.host)!.className).toBe("v22");
    expect(look(l.host)!.getAttribute("data-state")).toBe("in");
    expect(count()).toBe(n);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("cut during a swap removes both hosts; the next take shows the new look", () => {
    for (const swap of ["crossfade", "outIn"] as const) {
      const { app, stage, l } = setup();
      pendingOut(l.root!);
      app.handle({ t: "layer", layer: variant(2, { swap }) });
      app.handle({ t: "cut" });
      expect(hosts(stage).length).toBe(0);
      expect(vi.getTimerCount()).toBe(0);
      app.handle({ t: "state", layerId: "lt", state: "in" });
      expect(hosts(stage)).toEqual([l.host]);
      expect(look(l.host)!.className).toBe("v2");
      expect(look(l.host)!.getAttribute("data-state")).toBe("in");
      document.body.replaceChildren();
    }
  });

  it("clear or remove during a swap leaves nothing behind", () => {
    const { app, stage } = setup();
    app.handle({ t: "layer", layer: variant(2, { swap: "crossfade" }) });
    app.handle({ t: "clear" }); // no durations in jsdom → out at once
    expect(hosts(stage).length).toBe(0);
    app.handle({ t: "state", layerId: "lt", state: "in" });
    const l = app.layers.get("lt")!;
    pendingOut(l.root!);
    app.handle({ t: "layer", layer: variant(3, { swap: "outIn" }) });
    app.handle({ t: "remove", layerId: "lt" });
    expect(hosts(stage).length).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });
});

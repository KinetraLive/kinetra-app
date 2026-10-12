import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./app";
import { setPreview } from "./layer";
import type { ScreenInfo, ServerMsg, WireLayer } from "./protocol";
import { Stage } from "./stage";
import { isPreview, screenId, wsUrl } from "./url";

const screen: ScreenInfo = { id: "test", name: "Test", width: 1920, height: 1080, background: "transparent" };

function layer(id: string, over: Partial<WireLayer> = {}): WireLayer {
  return {
    id,
    z: 1,
    templateRev: 1,
    html: `<div class="${id}"><span class="name">{{name}}</span></div>`,
    css: "",
    fields: { name: id.toUpperCase() },
    fieldTypes: { name: "text" },
    state: "in",
    ...over,
  };
}

type Kind = "cut" | "crossfade" | "outIn";
const take = (kind: Kind, layers: WireLayer[], hide: string[] = [], ms = 400): ServerMsg => ({
  t: "take",
  transition: { kind, ms },
  layers,
  hide,
});

function setup(layers = [layer("a"), layer("b")]) {
  const stage = new Stage(document.body);
  const app = new App(stage);
  app.handle({ t: "snapshot", screen, layers });
  return { app, stage };
}

/** Sends `m` and waits for the studio chunk. */
async function send(app: App, ...ms: ServerMsg[]) {
  for (const m of ms) app.handle(m);
  await vi.dynamicImportSettled();
}

const hosts = (s: Stage) => Array.from(s.box.children) as HTMLElement[];
const text = (h: HTMLElement) => h.shadowRoot!.querySelector(".name")!.textContent;
const root = (h: HTMLElement) => h.shadowRoot!.querySelector("[data-state]");
const ops = (hs: HTMLElement[]) => hs.map((h) => h.style.opacity);

/** Makes every root report one finite (finish spy) and one endless animation. */
function stubAnims() {
  const fin = vi.fn();
  const loop = vi.fn();
  const proto = Element.prototype as unknown as { getAnimations?: unknown };
  proto.getAnimations = () => [
    { effect: { getComputedTiming: () => ({ iterations: 1 }) }, finish: fin, finished: new Promise(() => {}) },
    { effect: { getComputedTiming: () => ({ iterations: Infinity }) }, finish: loop, finished: new Promise(() => {}) },
  ];
  return { fin, loop, restore: () => delete proto.getAnimations };
}

/** Makes `el`'s out-animation pending until the returned function is called. */
function pendingOut(el: Element) {
  let finish!: () => void;
  const finished = new Promise<void>((r) => (finish = r));
  el.getAnimations = () => [{ finished } as unknown as Animation];
  return finish;
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  setPreview(false);
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

describe("preview bus", () => {
  it("URL: /screen/<id>/preview connects with bus=preview", () => {
    expect(isPreview("/screen/main/preview")).toBe(true);
    expect(isPreview("/screen/main/preview/")).toBe(true);
    expect(isPreview("/screen/main")).toBe(false);
    expect(isPreview("/screen/preview")).toBe(false);
    expect(screenId("/screen/main/preview")).toBe("main");
    const loc = { protocol: "http:", host: "127.0.0.1:7770" };
    expect(wsUrl(loc, "main", true)).toBe("ws://127.0.0.1:7770/ws?screen=main&bus=preview");
    expect(wsUrl(loc, "main")).toBe("ws://127.0.0.1:7770/ws?screen=main");
  });

  it("shows layers settled: finite in-animations finished, loops run; out removes at once", () => {
    setPreview(true);
    const a = stubAnims();
    try {
      const { app, stage } = setup([layer("a", { state: "out" })]);
      app.handle({ t: "state", layerId: "a", state: "in" });
      const h = app.layers.get("a")!.host;
      expect(hosts(stage)).toEqual([h]);
      expect(root(h)!.getAttribute("data-state")).toBe("in");
      expect(a.fin).toHaveBeenCalled();
      expect(a.loop).not.toHaveBeenCalled();
      // Going out: removed now, though the out-animation would still be running.
      app.handle({ t: "state", layerId: "a", state: "out" });
      expect(hosts(stage)).toEqual([]);
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      a.restore();
    }
  });

  it("a variant swap is settled in place (one host), clear removes at once", () => {
    setPreview(true);
    const { app, stage } = setup();
    const h = app.layers.get("a")!.host;
    app.handle({ t: "layer", layer: layer("a", { templateRev: 2, html: '<p class="v2">{{name}}</p>', swap: "crossfade" }) });
    expect(hosts(stage).length).toBe(2);
    expect(app.layers.get("a")!.host).toBe(h);
    pendingOut(app.layers.get("b")!.root!);
    app.handle({ t: "clear" });
    expect(hosts(stage)).toEqual([]);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("a take on the preview bus is a cut", async () => {
    setPreview(true);
    const { app, stage } = setup();
    await send(app, take("crossfade", [layer("c")], ["b"]));
    expect(hosts(stage).map(text)).toEqual(["A", "C"]);
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe("take", () => {
  it("cut: incoming settled at once, outgoing removed, changed layers patched in place", async () => {
    const { app, stage } = setup();
    const ha = app.layers.get("a")!.host;
    pendingOut(app.layers.get("b")!.root!); // an out-animation would be pending: the cut ignores it
    const a = stubAnims();
    try {
      await send(app, take("cut", [layer("a", { fields: { name: "Grace" } }), layer("c")], ["b"]));
      const hc = app.layers.get("c")!.host;
      expect(hosts(stage)).toEqual([ha, hc]);
      expect(text(ha)).toBe("Grace");
      expect(root(hc)!.getAttribute("data-state")).toBe("in");
      expect(a.fin).toHaveBeenCalled(); // settled
      expect(app.layers.get("b")!.state).toBe(null);
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      a.restore();
    }
  });

  it("crossfade: both hosts during the fade, one after", async () => {
    const { app, stage } = setup();
    const oldA = app.layers.get("a")!;
    const hb = app.layers.get("b")!.host;
    await send(app, take("crossfade", [layer("a", { fields: { name: "Grace" } }), layer("c")], ["b"], 400));
    const na = app.layers.get("a")!;
    const hc = app.layers.get("c")!.host;
    expect(na).not.toBe(oldA); // Program has the new content at once
    expect(hosts(stage)).toEqual([oldA.host, hb, na.host, hc]);
    expect(ops([na.host, hc])).toEqual(["0", "0"]);
    expect(text(na.host)).toBe("Grace");
    // Patches during the fade go to the new look.
    app.handle({ t: "fields", layerId: "a", fields: { name: "Linus" } });
    expect(text(na.host)).toBe("Linus");
    vi.advanceTimersByTime(420);
    expect(hosts(stage)).toEqual([na.host, hc]);
    expect(ops([na.host, hc])).toEqual(["", ""]);
    expect(app.layers.get("b")!.state).toBe(null);
    expect(hb.style.opacity).toBe("");
    expect(vi.getTimerCount()).toBe(0);
  });

  it("crossfade animates host opacity only, over ms, and drops the fills", async () => {
    const calls: { el: Element; k: unknown; o: KeyframeAnimationOptions; cancel: () => void }[] = [];
    const proto = HTMLElement.prototype as unknown as { animate?: unknown };
    proto.animate = function (this: Element, k: unknown, o: KeyframeAnimationOptions) {
      const c = { el: this, k, o, cancel: vi.fn() };
      calls.push(c);
      return c;
    };
    try {
      const { app } = setup();
      const hb = app.layers.get("b")!.host;
      await send(app, take("crossfade", [layer("c")], ["b"], 600));
      const hc = app.layers.get("c")!.host;
      expect(calls.map((c) => [c.el, c.k, c.o.duration])).toEqual([
        [hc, { opacity: [0, 1] }, 600],
        [hb, { opacity: [1, 0] }, 600],
      ]);
      vi.advanceTimersByTime(620);
      expect(calls.every((c) => (c.cancel as ReturnType<typeof vi.fn>).mock.calls.length === 1)).toBe(true);
    } finally {
      delete proto.animate;
    }
  });

  it("crossfade ignores reduced motion: on-air graphics always animate", async () => {
    vi.stubGlobal("matchMedia", (q: string) => ({ matches: q.includes("reduce") }));
    const { app, stage } = setup();
    const hb = app.layers.get("b")!.host;
    await send(app, take("crossfade", [layer("c")], ["b"], 400));
    const hc = app.layers.get("c")!.host;
    // Both looks are on screen, mid-fade.
    expect(hosts(stage)).toContain(hb);
    expect(hosts(stage)).toContain(hc);
    expect(vi.getTimerCount()).toBeGreaterThan(0);
    vi.advanceTimersByTime(420);
    expect(hosts(stage).map(text)).toEqual(["A", "C"]);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("outIn: outgoing play out, incoming play in; a changed on-air layer plays out, then in", async () => {
    const { app, stage } = setup();
    const la = app.layers.get("a")!;
    const oldA = la.host;
    const hb = app.layers.get("b")!.host;
    const outA = pendingOut(la.root!);
    const outB = pendingOut(app.layers.get("b")!.root!);
    await send(app, take("outIn", [layer("a", { fields: { name: "Grace" } }), layer("c")], ["b"]));
    const hc = app.layers.get("c")!.host;
    expect(root(hb)!.getAttribute("data-state")).toBe("out");
    expect(root(oldA)!.getAttribute("data-state")).toBe("out");
    expect(root(hc)!.getAttribute("data-state")).toBe("in");
    expect(hosts(stage)).toEqual([oldA, hb, hc]);
    outA();
    outB();
    await vi.waitFor(() => expect(hosts(stage)).toEqual([hc, la.host]));
    expect(la.host).not.toBe(oldA);
    expect(text(la.host)).toBe("Grace");
    expect(root(la.host)!.getAttribute("data-state")).toBe("in");
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe("T-bar", () => {
  async function prepared() {
    const s = setup();
    const oldA = s.app.layers.get("a")!;
    const hb = s.app.layers.get("b")!.host;
    await send(s.app, { t: "prepare", layers: [layer("a", { fields: { name: "Grace" } }), layer("c")], hide: ["b"] });
    const [, , na, hc] = hosts(s.stage);
    return { ...s, oldA, hb, na, hc };
  }

  it("prepare mounts incoming hosts settled at 0; tbar sets opacities; commit finalises", async () => {
    const a = stubAnims();
    let p: Awaited<ReturnType<typeof prepared>>;
    try {
      p = await prepared();
      expect(a.fin).toHaveBeenCalled(); // settled
    } finally {
      a.restore();
    }
    const { app, stage, oldA, hb, na, hc } = p;
    expect(hosts(stage)).toEqual([oldA.host, hb, na, hc]);
    expect(ops([oldA.host, hb, na, hc])).toEqual(["", "", "0", "0"]);
    expect(text(na)).toBe("Grace");
    expect(app.layers.get("a")).toBe(oldA); // Program unchanged until commit
    expect(app.layers.has("c")).toBe(false);
    for (const t of [0.25, 0.5, 0.75]) {
      app.handle({ t: "tbar", p: t });
      expect(ops([oldA.host, hb, na, hc])).toEqual([1 - t, 1 - t, t, t].map(String));
    }
    app.handle({ t: "commit" });
    expect(hosts(stage)).toEqual([na, hc]);
    expect(ops([na, hc])).toEqual(["", ""]);
    expect(app.layers.get("a")!.host).toBe(na);
    expect(app.layers.get("c")!.host).toBe(hc);
    expect(app.layers.get("b")!.state).toBe(null);
    expect(hb.style.opacity).toBe("");
    expect(vi.getTimerCount()).toBe(0);
    // Later T-bar messages without a prepare do nothing.
    app.handle({ t: "tbar", p: 0.5 });
    expect(ops([na, hc])).toEqual(["", ""]);
  });

  it("abort removes the prepared hosts and restores Program", async () => {
    const { app, stage, oldA, hb } = await prepared();
    app.handle({ t: "tbar", p: 0.4 });
    app.handle({ t: "abort" });
    expect(hosts(stage)).toEqual([oldA.host, hb]);
    expect(ops([oldA.host, hb])).toEqual(["", ""]);
    expect(app.layers.get("a")).toBe(oldA);
    expect(text(oldA.host)).toBe("A");
    expect(app.layers.has("c")).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("tbar clamps p to 0..1", async () => {
    const { app, hb, hc } = await prepared();
    app.handle({ t: "tbar", p: 2 });
    expect(ops([hb, hc])).toEqual(["0", "1"]);
    app.handle({ t: "tbar", p: -1 });
    expect(ops([hb, hc])).toEqual(["1", "0"]);
  });

  it("a take during a prepared T-bar aborts it first", async () => {
    const { app, stage, oldA, hb, na, hc } = await prepared();
    app.handle({ t: "tbar", p: 0.5 });
    app.handle(take("cut", [layer("d")]));
    expect(na.isConnected || hc.isConnected).toBe(false);
    const hd = app.layers.get("d")!.host;
    expect(hosts(stage)).toEqual([oldA.host, hb, hd]);
    expect(ops([oldA.host, hb])).toEqual(["", ""]);
    // The aborted T-bar's commit is ignored.
    app.handle({ t: "commit" });
    expect(app.layers.get("a")).toBe(oldA);
  });

  it("PANIC during a prepared T-bar: aborted, then every layer cut", async () => {
    const { app, stage, oldA, hb } = await prepared();
    app.handle({ t: "tbar", p: 0.5 });
    app.handle({ t: "cut" });
    expect(hosts(stage)).toEqual([]);
    expect(ops([oldA.host, hb])).toEqual(["", ""]);
    expect(app.layers.has("c")).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("a snapshot during a prepared T-bar aborts it", async () => {
    const { app, stage, oldA, hb } = await prepared();
    app.handle({ t: "snapshot", screen, layers: [layer("a"), layer("b")] });
    expect(hosts(stage)).toEqual([oldA.host, hb]);
  });

  it("a snapshot during a crossfade finishes it", async () => {
    const { app, stage } = setup();
    await send(app, take("crossfade", [layer("c")], ["b"]));
    const hc = app.layers.get("c")!.host;
    app.handle({ t: "snapshot", screen, layers: [layer("a"), layer("b", { state: "out" }), layer("c")] });
    expect(hosts(stage).map(text)).toEqual(["A", "C"]);
    expect(app.layers.get("c")!.host).toBe(hc);
    expect(hc.style.opacity).toBe("");
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe("lazy studio chunk", () => {
  it("queues every message while it loads and applies them in order", async () => {
    const { app, stage } = setup();
    app.handle(take("cut", [layer("c")], ["b"]));
    app.handle({ t: "fields", layerId: "c", fields: { name: "Grace" } });
    app.handle({ t: "state", layerId: "a", state: "out" });
    // Nothing applied yet: the fields message must not run before the take.
    expect(hosts(stage).map(text)).toEqual(["A", "B"]);
    await vi.dynamicImportSettled();
    expect(text(app.layers.get("c")!.host)).toBe("Grace");
    expect(app.layers.get("b")!.state).toBe(null);
    await vi.waitFor(() => expect(hosts(stage).map(text)).toEqual(["Grace"]));
    expect(vi.getTimerCount()).toBe(0);
  });

  it("is not loaded without studio messages", async () => {
    const { app } = setup();
    app.handle({ t: "fields", layerId: "a", fields: { name: "Grace" } });
    expect((app as unknown as { st?: unknown; q?: unknown }).q).toBeUndefined();
    expect((app as unknown as { st?: unknown }).st).toBeUndefined();
  });
});

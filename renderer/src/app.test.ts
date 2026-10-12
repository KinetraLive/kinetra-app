import { afterEach, describe, expect, it, vi } from "vitest";
import { App } from "./app";
import type { ScreenInfo, ServerMsg, WireLayer } from "./protocol";
import { Stage } from "./stage";

const screen: ScreenInfo = { id: "test", name: "Test", width: 1920, height: 1080, background: "transparent" };

function layer(over: Partial<WireLayer> = {}): WireLayer {
  return {
    id: "lower-third",
    z: 1,
    templateRev: 1,
    html: '<div class="lt"><span class="name">{{name}}</span></div>',
    css: "",
    fields: { name: "Ada" },
    fieldTypes: { name: "text" },
    state: "in",
    ...over,
  };
}

function setup() {
  const stage = new Stage(document.body);
  const unknown = vi.fn();
  const app = new App(stage, unknown);
  return { app, stage, unknown };
}

afterEach(() => {
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

describe("App", () => {
  it("snapshot sizes the screen box and shows on-air layers", () => {
    const { app, stage } = setup();
    app.handle({ t: "snapshot", screen: { ...screen, background: "#123456" }, layers: [layer(), layer({ id: "off", state: "out" })] });
    expect(stage.box.style.width).toBe("1920px");
    expect(stage.box.style.height).toBe("1080px");
    expect(stage.box.style.background).not.toBe("");
    expect(stage.box.style.transform).toMatch(/scale\(/);
    expect(stage.box.children.length).toBe(1);
    expect(app.layers.size).toBe(2);
  });

  it("transparent background stays transparent", () => {
    const { app, stage } = setup();
    app.handle({ t: "snapshot", screen, layers: [] });
    expect(stage.box.style.background).toBe("");
  });

  it("an identical snapshot changes nothing on air", async () => {
    const { app } = setup();
    app.handle({ t: "snapshot", screen, layers: [layer()] });
    const l = app.layers.get("lower-third")!;
    const { host, root } = l;
    const text = l.shadow.querySelector(".name")!.firstChild;
    const records: MutationRecord[] = [];
    const mo = new MutationObserver((r) => records.push(...r));
    mo.observe(l.shadow, { subtree: true, attributes: true, childList: true, characterData: true });
    const setAttr = vi.spyOn(root!, "setAttribute");

    app.handle({ t: "snapshot", screen, layers: [layer()] });
    await Promise.resolve();
    mo.disconnect();
    records.push(...mo.takeRecords());

    expect(records).toEqual([]);
    expect(setAttr).not.toHaveBeenCalled();
    expect(app.layers.get("lower-third")!.host).toBe(host);
    expect(app.layers.get("lower-third")!.root).toBe(root);
    expect(app.layers.get("lower-third")!.shadow.querySelector(".name")!.firstChild).toBe(text);
    expect(root!.getAttribute("data-state")).toBe("in");
  });

  it("a snapshot with changed fields patches in place", () => {
    const { app } = setup();
    app.handle({ t: "snapshot", screen, layers: [layer()] });
    const root = app.layers.get("lower-third")!.root;
    app.handle({ t: "snapshot", screen, layers: [layer({ fields: { name: "Grace" } })] });
    expect(app.layers.get("lower-third")!.root).toBe(root);
    expect(root!.textContent).toBe("Grace");
  });

  it("a snapshot missing a layer removes it", () => {
    const { app, stage } = setup();
    app.handle({ t: "snapshot", screen, layers: [layer(), layer({ id: "b" })] });
    expect(stage.box.children.length).toBe(2);
    app.handle({ t: "snapshot", screen, layers: [layer({ id: "b" })] });
    expect(app.layers.has("lower-third")).toBe(false);
    expect(stage.box.children.length).toBe(1);
  });

  it("a snapshot with a new templateRev recompiles", () => {
    const { app } = setup();
    app.handle({ t: "snapshot", screen, layers: [layer()] });
    const old = app.layers.get("lower-third")!.root;
    app.handle({ t: "snapshot", screen, layers: [layer({ templateRev: 2, html: '<p class="v2">{{name}}</p>' })] });
    const l = app.layers.get("lower-third")!;
    expect(l.root).not.toBe(old);
    expect(l.root!.className).toBe("v2");
    expect(l.root!.getAttribute("data-state")).toBe("in");
  });

  it("cut takes every layer off at once, skipping the out-animation, and keeps it compiled", () => {
    const { app, stage } = setup();
    app.handle({ t: "snapshot", screen, layers: [layer(), layer({ id: "b", z: 2 })] });
    expect(stage.box.children.length).toBe(2);
    // An out-animation that never finishes: `clear` would wait for it, `cut` must not.
    const pending = { finished: new Promise(() => {}) } as unknown as Animation;
    for (const l of app.layers.values()) l.root!.getAnimations = () => [pending];
    const nodes = [...app.layers.values()].map((l) => l.shadow.querySelector(".name"));
    app.handle({ t: "cut" });
    expect(stage.box.children.length).toBe(0);
    for (const l of app.layers.values()) expect(l.root!.getAttribute("data-state")).toBe("out");
    app.handle({ t: "state", layerId: "lower-third", state: "in" });
    const l = app.layers.get("lower-third")!;
    expect(stage.box.children.length).toBe(1);
    expect(l.root!.getAttribute("data-state")).toBe("in");
    expect(l.shadow.querySelector(".name")).toBe(nodes[0]); // not recompiled
  });

  it("handles layer/state/fields/remove/clear", () => {
    const { app, stage } = setup();
    app.handle({ t: "snapshot", screen, layers: [] });
    app.handle({ t: "layer", layer: layer({ state: "out" }) });
    expect(stage.box.children.length).toBe(0);
    app.handle({ t: "state", layerId: "lower-third", state: "in" });
    const l = app.layers.get("lower-third")!;
    expect(l.root!.getAttribute("data-state")).toBe("in");
    app.handle({ t: "fields", layerId: "lower-third", fields: { name: "Grace" } });
    expect(l.root!.textContent).toBe("Grace");
    app.handle({ t: "clear" });
    expect(stage.box.children.length).toBe(0); // no durations in jsdom → immediate
    app.handle({ t: "state", layerId: "lower-third", state: "in" });
    app.handle({ t: "remove", layerId: "lower-third" });
    expect(app.layers.size).toBe(0);
    expect(stage.box.children.length).toBe(0);
    // Messages for unknown layers are no-ops.
    app.handle({ t: "fields", layerId: "nope", fields: { a: "b" } });
    app.handle({ t: "state", layerId: "nope", state: "in" });
  });

  it("unknown message types are ignored", () => {
    const { app } = setup();
    expect(() => app.handle({ t: "bogus" } as unknown as ServerMsg)).not.toThrow();
  });

  it("another protocol version warns and continues", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const { app } = setup();
    app.handle({ t: "hello", protocol: 99 });
    expect(warn).toHaveBeenCalled();
    app.handle({ t: "snapshot", screen, layers: [layer()] });
    expect(app.layers.size).toBe(1);
  });

  it("unknownScreen stays transparent and reports", () => {
    const err = vi.spyOn(console, "error");
    const { app, stage, unknown } = setup();
    app.handle({ t: "error", code: "unknownScreen" });
    expect(unknown).toHaveBeenCalled();
    expect(stage.box.children.length).toBe(0);
    expect(err).not.toHaveBeenCalled();
  });

  it("a broken layer does not break the others", () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const { app } = setup();
    const bad = layer({ id: "bad" });
    Object.defineProperty(bad, "fields", { get: () => { throw new Error("boom"); } });
    app.handle({ t: "snapshot", screen, layers: [bad, layer({ id: "good" })] });
    expect(app.layers.get("good")!.root!.getAttribute("data-state")).toBe("in");
  });
});

describe("layer boxes (rect)", () => {
  /** Watches a layer for any DOM change inside its shadow root and for data-state writes. */
  function watch(l: { shadow: ShadowRoot; root: Element | null }) {
    const records: MutationRecord[] = [];
    const mo = new MutationObserver((r) => records.push(...r));
    mo.observe(l.shadow, { subtree: true, attributes: true, childList: true, characterData: true });
    const setAttr = vi.spyOn(l.root!, "setAttribute");
    const anim = vi.fn();
    l.root!.addEventListener("animationstart", anim);
    return {
      done: async () => {
        await Promise.resolve();
        mo.disconnect();
        records.push(...mo.takeRecords());
        return { records, setAttr, anim };
      },
    };
  }

  const box = (h: HTMLElement) => [h.style.left, h.style.top, h.style.width, h.style.height];

  it("positions the host at its rect, overflow visible", () => {
    const { app } = setup();
    app.handle({ t: "snapshot", screen, layers: [layer({ rect: { x: 100, y: 800, w: 900, h: 160 } })] });
    const h = app.layers.get("lower-third")!.host;
    expect(h.style.position).toBe("absolute");
    expect(box(h)).toEqual(["100px", "800px", "900px", "160px"]);
    expect(h.style.overflow).toBe("");
    expect(h.style.transform).toBe("");
  });

  it("an absent rect is the full screen", () => {
    const { app } = setup();
    app.handle({ t: "snapshot", screen, layers: [layer()] });
    const h = app.layers.get("lower-third")!.host;
    expect(box(h)).toEqual(["0px", "0px", "100%", "100%"]);
  });

  it("a rect message only moves the host: same nodes, no replay, no recompile", async () => {
    const { app } = setup();
    app.handle({ t: "snapshot", screen, layers: [layer({ rect: { x: 0, y: 0, w: 10, h: 10 } })] });
    const l = app.layers.get("lower-third")!;
    const { host, root } = l;
    const text = l.shadow.querySelector(".name")!.firstChild;
    const w = watch(l);

    app.handle({ t: "rect", layerId: "lower-third", rect: { x: 40, y: 50, w: 600, h: 120.5 } });
    const { records, setAttr, anim } = await w.done();

    expect(box(host)).toEqual(["40px", "50px", "600px", "120.5px"]);
    expect(records).toEqual([]);
    expect(setAttr).not.toHaveBeenCalled();
    expect(anim).not.toHaveBeenCalled();
    expect(l.root).toBe(root);
    expect(l.shadow.querySelector(".name")!.firstChild).toBe(text);
    expect(root!.getAttribute("data-state")).toBe("in");
    expect(host.isConnected).toBe(true);
  });

  it("rect for an unknown layer, or with invalid numbers, is ignored", () => {
    const { app } = setup();
    app.handle({ t: "snapshot", screen, layers: [layer({ rect: { x: 1, y: 2, w: 3, h: 4 } })] });
    const h = app.layers.get("lower-third")!.host;
    expect(() => app.handle({ t: "rect", layerId: "nope", rect: { x: 1, y: 1, w: 1, h: 1 } })).not.toThrow();
    app.handle({ t: "rect", layerId: "lower-third", rect: { x: NaN, y: 0, w: 1, h: 1 } });
    app.handle({ t: "rect", layerId: "lower-third", rect: { x: 0, y: 0, w: Infinity, h: 1 } });
    app.handle({ t: "rect", layerId: "lower-third", rect: { x: "5", y: 0, w: 1, h: 1 } as never });
    app.handle({ t: "rect", layerId: "lower-third" } as unknown as ServerMsg);
    expect(box(h)).toEqual(["1px", "2px", "3px", "4px"]);
  });

  it("a same-rev layer message updates z, rect and fields in place", async () => {
    const { app } = setup();
    app.handle({ t: "snapshot", screen, layers: [layer()] });
    const l = app.layers.get("lower-third")!;
    const { host, root } = l;
    const span = l.shadow.querySelector(".name")!;
    const setAttr = vi.spyOn(root!, "setAttribute");

    app.handle({
      t: "layer",
      layer: layer({ z: 7, rect: { x: 10, y: 20, w: 30, h: 40 }, fields: { name: "Grace" }, vars: { accent: "#ff0000" } }),
    });

    expect(app.layers.get("lower-third")).toBe(l);
    expect(l.host).toBe(host);
    expect(l.root).toBe(root);
    expect(l.shadow.querySelector(".name")).toBe(span);
    expect(span.textContent).toBe("Grace");
    expect(host.style.zIndex).toBe("7");
    expect(box(host)).toEqual(["10px", "20px", "30px", "40px"]);
    expect(host.style.getPropertyValue("--accent")).toBe("#ff0000");
    expect(setAttr).not.toHaveBeenCalled();
    expect(root!.getAttribute("data-state")).toBe("in");

    // Dropping the rect goes back to the full screen; state transitions still work.
    app.handle({ t: "layer", layer: layer({ z: 7, state: "out" }) });
    expect(box(host)).toEqual(["0px", "0px", "100%", "100%"]);
    expect(host.isConnected).toBe(false); // no durations in jsdom → immediate
  });

  it("a layer message with a new rev recompiles at the new rect", () => {
    const { app } = setup();
    app.handle({ t: "snapshot", screen, layers: [layer()] });
    const old = app.layers.get("lower-third")!.root;
    app.handle({
      t: "layer",
      layer: layer({ templateRev: 2, html: '<p class="v2">{{name}}</p>', rect: { x: 5, y: 6, w: 7, h: 8 } }),
    });
    const l = app.layers.get("lower-third")!;
    expect(l.root).not.toBe(old);
    expect(l.root!.className).toBe("v2");
    expect(l.root!.getAttribute("data-state")).toBe("in");
    expect(box(l.host)).toEqual(["5px", "6px", "7px", "8px"]);
  });

  it("a reconnect snapshot with a changed rect/z and the same rev does not replay", async () => {
    const { app } = setup();
    app.handle({ t: "snapshot", screen, layers: [layer({ rect: { x: 0, y: 0, w: 100, h: 100 } })] });
    const l = app.layers.get("lower-third")!;
    const { host, root } = l;
    const w = watch(l);

    app.handle({ t: "snapshot", screen, layers: [layer({ z: 3, rect: { x: 50, y: 60, w: 200, h: 80 } })] });
    const { records, setAttr, anim } = await w.done();

    expect(app.layers.get("lower-third")).toBe(l);
    expect(l.root).toBe(root);
    expect(records).toEqual([]);
    expect(setAttr).not.toHaveBeenCalled();
    expect(anim).not.toHaveBeenCalled();
    expect(host.style.zIndex).toBe("3");
    expect(box(host)).toEqual(["50px", "60px", "200px", "80px"]);
  });

  it("show/hide cycles with rect moves keep the node count flat", () => {
    const { app, stage } = setup();
    app.handle({ t: "snapshot", screen, layers: [layer({ state: "out" })] });
    const count = () => stage.box.querySelectorAll("*").length + app.layers.get("lower-third")!.shadow.querySelectorAll("*").length;
    app.handle({ t: "state", layerId: "lower-third", state: "in" });
    const n = count();
    for (let i = 0; i < 20; i++) {
      app.handle({ t: "rect", layerId: "lower-third", rect: { x: i, y: i, w: 100 + i, h: 50 } });
      app.handle({ t: "state", layerId: "lower-third", state: "out" });
      app.handle({ t: "state", layerId: "lower-third", state: "in" });
    }
    expect(count()).toBe(n);
  });
});

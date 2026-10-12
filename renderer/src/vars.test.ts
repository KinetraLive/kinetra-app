import { afterEach, describe, expect, it, vi } from "vitest";
import { App } from "./app";
import type { ScreenInfo, WireLayer } from "./protocol";
import { Stage } from "./stage";

const screen: ScreenInfo = { id: "s", name: "S", width: 1920, height: 1080, background: "transparent" };

function layer(over: Partial<WireLayer> = {}): WireLayer {
  return {
    id: "lt",
    z: 1,
    templateRev: 1,
    html: '<div class="lt"><span class="name">{{name}}</span></div>',
    css: ".lt{color:var(--accent)}",
    fields: { name: "Ada" },
    fieldTypes: { name: "text" },
    state: "in",
    vars: { accent: "#ff0000", radius: "4px" },
    ...over,
  };
}

function setup() {
  const app = new App(new Stage(document.body));
  app.handle({ t: "snapshot", screen, layers: [layer()] });
  const l = app.layers.get("lt")!;
  return { app, l, host: l.host, root: l.root!, text: l.root!.querySelector(".name")!.firstChild! };
}

/** Records data-state changes and child-list changes inside the shadow root. */
function watch(l: { shadow: ShadowRoot }) {
  const recs: MutationRecord[] = [];
  const mo = new MutationObserver((r) => recs.push(...r));
  mo.observe(l.shadow, { subtree: true, childList: true, attributes: true, attributeFilter: ["data-state"] });
  return () => {
    recs.push(...mo.takeRecords());
    mo.disconnect();
    return recs;
  };
}

afterEach(() => {
  vi.restoreAllMocks();
  document.body.replaceChildren();
  document.head.querySelectorAll("link[data-font]").forEach((n) => n.remove());
});

describe("template variables", () => {
  it("a new layer gets its vars as custom properties on the host", () => {
    const { host } = setup();
    expect(host.style.getPropertyValue("--accent")).toBe("#ff0000");
    expect(host.style.getPropertyValue("--radius")).toBe("4px");
    expect(host.style.position).toBe("absolute");
  });

  it("a vars message patches only the changed keys, keeping nodes and data-state", () => {
    const { app, l, host, root, text } = setup();
    const set = vi.spyOn(host.style, "setProperty");
    const attr = vi.spyOn(root, "setAttribute");
    const done = watch(l);
    app.handle({ t: "vars", layerId: "lt", vars: { accent: "#00ff00", radius: "4px" } });
    expect(set).toHaveBeenCalledTimes(1);
    expect(set).toHaveBeenCalledWith("--accent", "#00ff00");
    expect(host.style.getPropertyValue("--accent")).toBe("#00ff00");
    expect(host.style.getPropertyValue("--radius")).toBe("4px");
    expect(done()).toEqual([]);
    expect(attr).not.toHaveBeenCalled();
    expect(l.root).toBe(root);
    expect(root.querySelector(".name")!.firstChild).toBe(text);
    expect(root.getAttribute("data-state")).toBe("in");
  });

  it("ignores invalid keys and non-string values", () => {
    const { app, host } = setup();
    const bad: Record<string, string> = { Bad: "1", "1x": "1", "a b": "1", constructor: "2", n: 5 as unknown as string };
    bad["x".repeat(33)] = "1";
    app.handle({ t: "vars", layerId: "lt", vars: bad });
    expect(host.style.getPropertyValue("--Bad")).toBe("");
    expect(host.style.getPropertyValue("--n")).toBe("");
    expect(host.style.getPropertyValue("--constructor")).toBe("2");
    expect([...app.layers.get("lt")!.vars.keys()].sort()).toEqual(["accent", "constructor", "radius"]);
  });

  it("a newer full layer without a key removes it; unchanged vars are untouched", () => {
    const { app, l, host, root } = setup();
    const set = vi.spyOn(host.style, "setProperty");
    const done = watch(l);
    app.handle({ t: "snapshot", screen, layers: [layer({ vars: { accent: "#ff0000" } })] });
    expect(host.style.getPropertyValue("--radius")).toBe("");
    expect(host.style.getPropertyValue("--accent")).toBe("#ff0000");
    expect(set).not.toHaveBeenCalled();
    expect(done()).toEqual([]);
    expect(l.root).toBe(root);
  });

  it("a layer without vars (older server) removes all of them", () => {
    const { app, host } = setup();
    app.handle({ t: "layer", layer: layer({ vars: undefined }) });
    expect(host.style.getPropertyValue("--accent")).toBe("");
  });

  it("vars survive a recompile (new templateRev) on the same host", () => {
    const { app, host } = setup();
    app.handle({ t: "layer", layer: layer({ templateRev: 2, html: "<b>{{name}}</b>" }) });
    expect(app.layers.get("lt")!.host).toBe(host);
    expect(host.style.getPropertyValue("--accent")).toBe("#ff0000");
  });

  it("vars for an unknown layer are ignored", () => {
    const { app } = setup();
    expect(() => app.handle({ t: "vars", layerId: "nope", vars: { a: "1" } })).not.toThrow();
  });
});

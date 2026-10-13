// `gone`: the renderer reports a layer's finished out-animation (remote control's exact on_air).
import { afterEach, describe, expect, it, vi } from "vitest";

const sent = vi.hoisted(() => [] as unknown[]);
vi.mock("./connection", () => ({ tell: (m: unknown) => sent.push(m) }));

import { Layer } from "./layer";
import type { WireLayer } from "./protocol";

function def(over: Partial<WireLayer> = {}): WireLayer {
  return {
    id: "l1",
    z: 1,
    templateRev: 1,
    html: '<div class="lt"><span class="name">{{name}}</span></div>',
    css: ".lt{color:red}",
    fields: { name: "Ada" },
    fieldTypes: { name: "text" },
    state: "in",
    ...over,
  };
}

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
  sent.length = 0;
});

describe("gone", () => {
  it("is sent once the out-animation ends, not before", () => {
    fakeDuration("0.5s");
    vi.useFakeTimers();
    const l = new Layer(def(), document.createElement("div"));
    l.setState("out");
    expect(sent).toEqual([]);
    l.root!.dispatchEvent(new Event("animationend", { bubbles: true }));
    expect(sent).toEqual([{ t: "gone", layerId: "l1" }]);
  });

  it("is sent from the fallback timer too", () => {
    fakeDuration("0.3s");
    vi.useFakeTimers();
    const l = new Layer(def(), document.createElement("div"));
    l.setState("out");
    vi.advanceTimersByTime(349);
    expect(sent).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(sent).toEqual([{ t: "gone", layerId: "l1" }]);
  });

  it("is not sent for a cut or when the layer is taken again during the out-animation", () => {
    fakeDuration("0.5s");
    vi.useFakeTimers();
    const a = new Layer(def(), document.createElement("div"));
    a.cut();
    const b = new Layer(def({ id: "l2" }), document.createElement("div"));
    b.setState("out");
    b.setState("in");
    vi.advanceTimersByTime(5000);
    expect(sent).toEqual([]);
  });
});

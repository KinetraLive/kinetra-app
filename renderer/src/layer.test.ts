import { afterEach, describe, expect, it, vi } from "vitest";
import { Layer, compile, outDuration, parseTimes, safeAttr, styleVal, useWebImages, viaKinetra } from "./layer";
import type { FieldType, Fields, WireLayer } from "./protocol";

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
    html: '<div class="lt"><span class="name">{{name}}</span></div>',
    css: ".lt{color:red}",
    fields: { name: "Ada" },
    fieldTypes: { name: "text" },
    state: "out",
    ...over,
  };
}

/** Makes every element report the given computed animation duration (jsdom computes none). */
function fakeDuration(dur: string) {
  const real = window.getComputedStyle;
  return vi.spyOn(window, "getComputedStyle").mockImplementation((el: Element) => {
    const cs = real(el);
    return new Proxy(cs, {
      get: (t, p) =>
        p === "animationDuration" ? dur : p === "animationDelay" ? "0s" : Reflect.get(t, p, t),
    });
  });
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
  document.body.replaceChildren();
});

describe("compile", () => {
  it("binds {{key}} in text and attributes, with whitespace in braces", () => {
    const { div } = render('<p class="x {{cls}}">Hi {{ name }}!{{missing}}</p>', { name: "Ada", cls: "big" });
    const p = div.querySelector("p")!;
    expect(p.textContent).toBe("Hi Ada!");
    expect(p.getAttribute("class")).toBe("x big");
  });

  it("unknown keys render as empty", () => {
    const { div } = render('<p title="a{{nope}}b">{{nope}}</p>', {});
    expect(div.querySelector("p")!.getAttribute("title")).toBe("ab");
    expect(div.textContent).toBe("");
  });

  it("the template root is the first element", () => {
    const { root } = render("  <!-- c --> text <section>{{a}}</section><div></div>", { a: "1" });
    expect(root?.localName).toBe("section");
  });

  it("text fields are escaped; html fields render markup", () => {
    const evil = "<img src=x onerror=alert(1)>";
    const { div } = render("<p>{{t}}</p><p>{{h}}</p>", { t: evil, h: "<b>bold</b>" }, { t: "text", h: "html" });
    const [a, b] = div.querySelectorAll("p");
    expect(a.textContent).toBe(evil);
    expect(a.querySelector("img")).toBeNull();
    expect(b.querySelector("b")?.textContent).toBe("bold");
  });

  it("html fields drop scripts and event handlers", () => {
    const { div } = render("<p>{{h}}</p>", { h: '<i onclick="x()">a</i><script>x()</script>' }, { h: "html" });
    expect(div.querySelector("script")).toBeNull();
    expect(div.querySelector("i")!.hasAttribute("onclick")).toBe(false);
  });

  it("never binds into on* attributes", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const { div } = render('<button onclick="go({{x}})">b</button>', { x: "alert(1)" });
    expect(div.querySelector("button")!.getAttribute("onclick")).toBe("go({{x}})");
    expect(warn).toHaveBeenCalled();
  });

  it("filters URL attributes", () => {
    const html = '<a href="{{url}}">a</a>';
    expect(render(html, { url: "javascript:alert(1)" }).div.querySelector("a")!.getAttribute("href")).toBe("");
    expect(render(html, { url: " java\tscript:alert(1)" }).div.querySelector("a")!.getAttribute("href")).toBe("");
    // Outside renderer pages (the app's thumbnails) web links stay as they are.
    expect(render(html, { url: "https://x" }).div.querySelector("a")!.getAttribute("href")).toBe("https://x");
    useWebImages();
    expect(render(html, { url: "https://x" }).div.querySelector("a")!.getAttribute("href")).toBe("/web-image?u=https%3A%2F%2Fx");
    expect(render(html, { url: "/rel/a.png" }).div.querySelector("a")!.getAttribute("href")).toBe("/rel/a.png");
    const img = render('<img src="{{s}}">', { s: "data:image/png;base64,AAAA" }).div.querySelector("img")!;
    expect(img.getAttribute("src")).toBe("data:image/png;base64,AAAA");
    expect(safeAttr("src", "data:text/html,<script>")).toBe("");
    expect(safeAttr("srcset", "a.png 1x, javascript:x 2x")).toBe("");
    expect(safeAttr("title", "javascript:x")).toBe("javascript:x");
  });

  it("loads bound web images through Kinetra's copy, never from the internet", () => {
    const img = (s: string) => render('<img class="pic" src="{{s}}">', { s }).div.querySelector("img")!.getAttribute("src");
    expect(img("https://cdn.example/a b.png?w=2&h=3")).toBe("/web-image?u=https%3A%2F%2Fcdn.example%2Fa%20b.png%3Fw%3D2%26h%3D3");
    expect(img(" HTTP://x/y.png ")).toBe("/web-image?u=HTTP%3A%2F%2Fx%2Fy.png");
    expect(img("/assets/logo.png")).toBe("/assets/logo.png");
    expect(img("data:image/png;base64,AAAA")).toBe("data:image/png;base64,AAAA");
    expect(img("javascript:alert(1)")).toBe("");
    expect(img("file:///C:/x.png")).toBe("");
    expect(viaKinetra("srcset", "https://a/1.png 1x, /assets/2.png 2x")).toBe("/web-image?u=https%3A%2F%2Fa%2F1.png 1x, /assets/2.png 2x");
    expect(viaKinetra("title", "https://a")).toBe("https://a");
  });

  it("accepts only plain values in style attributes", () => {
    for (const ok of ["#e6c84a", "#FFF", "42", "0.5", "-3.25", "75%", "12px", "1.5em", "red", "dark blue", "+2", ""]) {
      expect(styleVal(ok)).toBe(ok);
    }
    for (const bad of [
      "url(https://evil.example/x.png)",
      "red;background:url(//evil)",
      "red; position: fixed",
      'red" onload="x',
      "red' x",
      "expression(alert(1))",
      'image-set("a.png" 1x)',
      "var(--x)",
      "calc(1px + 2px)",
      "}*{display:none",
      "\\75 rl(x)",
      "red/**/",
      "@import x",
      "a:b",
      "red\nblue",
      "<b>",
      "red !important",
      "x".repeat(65),
    ]) {
      expect(styleVal(bad)).toBe("");
    }
  });

  it("filters values bound into style attributes, keeping the static part", () => {
    const html = '<i style="color:{{c}}"></i><b style="--p:0{{p}}"></b>';
    let div = render(html, { c: "#e6c84a", p: "42" }).div;
    expect(div.querySelector("i")!.getAttribute("style")).toBe("color:#e6c84a");
    expect(div.querySelector("b")!.getAttribute("style")).toBe("--p:042");
    div = render(html, { c: "red;background:url(https://evil.example/a)", p: "1);background:url(x" }).div;
    expect(div.querySelector("i")!.getAttribute("style")).toBe("color:");
    expect(div.querySelector("b")!.getAttribute("style")).toBe("--p:0");
    // Other attributes keep their values as text.
    expect(render('<i title="{{c}}"></i>', { c: "a;b(c)" }).div.querySelector("i")!.getAttribute("title")).toBe("a;b(c)");
  });

  it("filters style bindings in list rows", () => {
    const { div } = render(
      '<ul><li data-each="l" style="color:{{.c}}">{{.t}}</li></ul>',
      { l: JSON.stringify([{ c: "#abc", t: "a" }, { c: "url(//evil)", t: "b" }]) },
      { l: "list" },
    );
    const li = div.querySelectorAll("li");
    expect((li[0] as HTMLElement).style.color).toBe("rgb(170, 187, 204)");
    expect((li[1] as HTMLElement).style.color).toBe("");
    expect(li[1].getAttribute("style")).not.toContain("url");
  });

  it("parses duration lists", () => {
    expect(parseTimes("0.3s, 200ms")).toEqual([300, 200]);
    expect(parseTimes("")).toEqual([0]);
  });
});

describe("Layer", () => {
  it("renders in an open shadow root with the template css", () => {
    const stage = document.createElement("div");
    const l = new Layer(def({ state: "in", z: 7 }), stage);
    expect(l.host.parentNode).toBe(stage);
    expect(l.host.style.zIndex).toBe("7");
    expect(l.shadow.querySelector("style")!.textContent).toBe(".lt{color:red}");
    expect(l.root!.getAttribute("data-state")).toBe("in");
  });

  it("a fields patch keeps the same Text node and root", () => {
    const l = new Layer(def({ state: "in" }), document.createElement("div"));
    const root = l.root!;
    const text = l.shadow.querySelector(".name")!.firstChild!;
    l.patch({ name: "Grace" });
    expect(l.root).toBe(root);
    expect(l.shadow.querySelector(".name")!.firstChild).toBe(text);
    expect(text.textContent).toBe("Grace");
  });

  it("an off-air layer is not in the DOM until shown", () => {
    const stage = document.createElement("div");
    const l = new Layer(def(), stage);
    expect(stage.childNodes.length).toBe(0);
    l.setState("in");
    expect(stage.contains(l.host)).toBe(true);
    expect(l.root!.getAttribute("data-state")).toBe("in");
  });

  it("hide without durations removes immediately", () => {
    const stage = document.createElement("div");
    const l = new Layer(def({ state: "in" }), stage);
    expect(outDuration(l.root!)).toBe(0);
    l.setState("out");
    expect(stage.contains(l.host)).toBe(false);
    expect(l.state).toBeNull();
  });

  it("hide sets data-state=out, then removes on animationend", () => {
    fakeDuration("0.5s");
    vi.useFakeTimers();
    const stage = document.createElement("div");
    const l = new Layer(def({ state: "in" }), stage);
    l.setState("out");
    expect(l.root!.getAttribute("data-state")).toBe("out");
    expect(stage.contains(l.host)).toBe(true);
    l.root!.dispatchEvent(new Event("animationend", { bubbles: true }));
    expect(stage.contains(l.host)).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("hide removes via the fallback timer (duration + 50 ms, capped at 3 s)", () => {
    fakeDuration("0.3s, 200ms");
    vi.useFakeTimers();
    const stage = document.createElement("div");
    const l = new Layer(def({ state: "in" }), stage);
    l.setState("out");
    vi.advanceTimersByTime(349);
    expect(stage.contains(l.host)).toBe(true);
    vi.advanceTimersByTime(1);
    expect(stage.contains(l.host)).toBe(false);

    vi.restoreAllMocks();
    fakeDuration("10s");
    l.setState("in");
    l.setState("out");
    vi.advanceTimersByTime(2999);
    expect(stage.contains(l.host)).toBe(true);
    vi.advanceTimersByTime(1);
    expect(stage.contains(l.host)).toBe(false);
  });

  it("show before removal cancels it", () => {
    fakeDuration("0.5s");
    vi.useFakeTimers();
    const stage = document.createElement("div");
    const l = new Layer(def({ state: "in" }), stage);
    const root = l.root!;
    l.setState("out");
    l.setState("in");
    root.dispatchEvent(new Event("animationend"));
    vi.advanceTimersByTime(5000);
    expect(stage.contains(l.host)).toBe(true);
    expect(root.getAttribute("data-state")).toBe("in");
    expect(l.root).toBe(root);
  });

  it("uses getAnimations().finished when supported", async () => {
    fakeDuration("1s");
    vi.useFakeTimers();
    const stage = document.createElement("div");
    const l = new Layer(def({ state: "in" }), stage);
    let finish!: () => void;
    const finished = new Promise<void>((r) => (finish = r));
    const proto = Element.prototype as unknown as { getAnimations?: unknown };
    const had = "getAnimations" in proto;
    const orig = proto.getAnimations;
    proto.getAnimations = () => [{ finished }];
    try {
      l.setState("out");
    } finally {
      if (had) proto.getAnimations = orig;
      else delete proto.getAnimations;
    }
    expect(stage.contains(l.host)).toBe(true);
    finish();
    await finished;
    await Promise.resolve();
    expect(stage.contains(l.host)).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("a new templateRev recompiles; on air it stays in", () => {
    const stage = document.createElement("div");
    const l = new Layer(def({ state: "in" }), stage);
    const old = l.root;
    l.apply(def({ state: "in", templateRev: 2, html: '<b class="v2">{{name}}</b>' }));
    expect(l.root).not.toBe(old);
    expect(l.shadow.querySelector(".v2")!.textContent).toBe("Ada");
    expect(l.root!.getAttribute("data-state")).toBe("in");
    expect(stage.contains(l.host)).toBe(true);
  });

  it("a new templateRev on air shows the new look settled: no replayed in-animation", () => {
    const stage = document.createElement("div");
    const l = new Layer(def({ state: "in" }), stage);
    const fin = vi.fn();
    const loop = vi.fn();
    const proto = Element.prototype as unknown as { getAnimations?: unknown };
    proto.getAnimations = () => [
      { effect: { getComputedTiming: () => ({ iterations: 1 }) }, finish: fin },
      { effect: { getComputedTiming: () => ({ iterations: Infinity }) }, finish: loop },
    ];
    try {
      l.apply(def({ state: "in", templateRev: 2, html: '<b class="v2">{{name}}</b>' }));
      expect(fin).toHaveBeenCalledTimes(1);
      expect(loop).not.toHaveBeenCalled();
      // Off air → on air with a new rev is a normal take: the in-animation plays.
      const off = new Layer(def({ state: "out" }), stage);
      fin.mockClear();
      off.apply(def({ state: "in", templateRev: 2 }));
      expect(fin).not.toHaveBeenCalled();
    } finally {
      delete proto.getAnimations;
    }
  });

  it("a template without elements does not throw", () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const stage = document.createElement("div");
    const l = new Layer(def({ state: "in", html: "just {{name}}" }), stage);
    expect(l.root).toBeNull();
    l.setState("out");
    expect(stage.contains(l.host)).toBe(false);
  });
});

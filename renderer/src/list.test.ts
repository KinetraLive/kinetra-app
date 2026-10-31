import { afterEach, describe, expect, it, vi } from "vitest";
import { App } from "./app";
import { Layer, compile, parseRows } from "./layer";
import type { FieldType, Fields, ScreenInfo, WireLayer } from "./protocol";
import { Stage } from "./stage";

const rows = (...r: Record<string, unknown>[]) => JSON.stringify(r);

const HTML =
  '<div class="board"><h1>{{title}}</h1><ol><li class="row" data-each="items"><b>{{#}}</b> <span class="n">{{.name}}</span> <i>{{.score}}</i> <em>{{title}}</em></li></ol><p>end</p></div>';

function def(over: Partial<WireLayer> = {}): WireLayer {
  return {
    id: "lb",
    z: 1,
    templateRev: 1,
    html: HTML,
    css: "",
    fields: { title: "Top", items: rows({ name: "Ada", score: "3" }, { name: "Grace", score: "2" }) },
    fieldTypes: { title: "text", items: "list" },
    state: "out",
    ...over,
  };
}

function render(html: string, fields: Fields, types: Record<string, FieldType> = {}) {
  const c = compile(html, types, (k) => fields[k] ?? "");
  const div = document.createElement("div");
  div.append(c.frag);
  return { ...c, div };
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

const lis = (l: Layer) => Array.from(l.shadow.querySelectorAll("li"));
const texts = (l: Layer) => lis(l).map((li) => li.textContent);

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
  document.body.replaceChildren();
});

describe("parseRows", () => {
  it("reads an array of row objects; anything else is no rows", () => {
    expect(parseRows(rows({ a: "x", b: 2, c: null, d: { x: 1 } }))).toEqual([{ a: "x", b: "2", c: "", d: "" }]);
    expect(parseRows("not json")).toEqual([]);
    expect(parseRows('{"a":1}')).toEqual([]);
    expect(parseRows("")).toEqual([]);
    expect(parseRows('[1, null, "x"]')).toEqual([{}, {}, {}]);
  });

  it("row lookups never reach Object.prototype", () => {
    const { div } = render('<ul><li data-each="l">{{.constructor}}|{{.toString}}</li></ul>', { l: rows({}) });
    expect(div.querySelector("li")!.textContent).toBe("|");
  });
});

describe("list fields", () => {
  it("clones the row per item with {{.col}}, {{#}}, --i and data-row, in place of the template", () => {
    const { div } = render(HTML, def().fields, def().fieldTypes);
    const ol = div.querySelector("ol")!;
    const items = Array.from(ol.querySelectorAll("li"));
    expect(items.map((li) => li.textContent)).toEqual(["1 Ada 3 Top", "2 Grace 2 Top"]);
    expect(items.map((li) => (li as HTMLElement).style.getPropertyValue("--i"))).toEqual(["0", "1"]);
    expect(items.map((li) => li.getAttribute("data-row"))).toEqual(["in", "in"]);
    // Anchor comment first, then the rows; the rest of the template is untouched.
    expect(ol.firstChild!.nodeType).toBe(8);
    expect(div.querySelector("p")!.textContent).toBe("end");
    expect(div.querySelector("h1")!.textContent).toBe("Top");
  });

  it("escapes cells and applies the URL and on* rules inside rows", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const evil = "<img src=x onerror=alert(1)>";
    const { div } = render(
      '<ul><li data-each="l"><a href="{{.url}}" onclick="go({{.x}})">{{.t}}</a></li></ul>',
      { l: rows({ t: evil, url: "javascript:alert(1)", x: "1" }, { t: "ok", url: "https://x/y", x: "2" }) },
      { l: "list" },
    );
    const [a, b] = div.querySelectorAll("a");
    expect(a.textContent).toBe(evil);
    expect(div.querySelector("img")).toBeNull();
    expect(a.getAttribute("href")).toBe("");
    expect(b.getAttribute("href")).toBe("/web-image?u=https%3A%2F%2Fx%2Fy"); // a web link goes through Kinetra
    expect(a.getAttribute("onclick")).toBe("go({{.x}})");
    expect(warn).toHaveBeenCalled();
  });

  it("data-limit caps the rows shown", () => {
    const { div } = render('<ul><li data-each="l" data-limit="2">{{.a}}</li></ul>', {
      l: rows({ a: "1" }, { a: "2" }, { a: "3" }),
    });
    expect(Array.from(div.querySelectorAll("li")).map((li) => li.textContent)).toEqual(["1", "2"]);
  });

  it("invalid JSON renders no rows and does not throw", () => {
    const { div } = render('<ul><li data-each="l">{{.a}}</li></ul>', { l: "[{oops" });
    expect(div.querySelectorAll("li").length).toBe(0);
  });

  it("the template root cannot be a row", () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const { div, root } = render('<div data-each="l">{{.a}}</div>', { l: rows({ a: "x" }) });
    expect(root).toBe(div.firstElementChild);
    expect(div.children.length).toBe(1);
  });

  it("patches rows in place by index: unchanged rows keep their nodes and are not touched", async () => {
    const l = new Layer(def({ state: "in" }), document.createElement("div"));
    const [r0, r1] = lis(l);
    const t0 = r0.querySelector(".n")!.firstChild;
    const records: MutationRecord[] = [];
    const mo = new MutationObserver((r) => records.push(...r));
    mo.observe(l.shadow, { subtree: true, attributes: true, childList: true, characterData: true });

    l.patch({ items: rows({ name: "Ada", score: "3" }, { name: "Grace", score: "9" }) });
    await Promise.resolve();
    mo.disconnect();
    records.push(...mo.takeRecords());

    expect(lis(l)).toEqual([r0, r1]);
    expect(r0.querySelector(".n")!.firstChild).toBe(t0);
    expect(texts(l)).toEqual(["1 Ada 3 Top", "2 Grace 9 Top"]);
    // Exactly one change: the score text of row 2.
    expect(records.length).toBe(1);
    expect(records[0].type).toBe("characterData");
    expect(records[0].target.parentNode).toBe(r1.querySelector("i"));
  });

  it("a bound style attribute on the row keeps --i across patches", () => {
    const l = new Layer(
      def({
        state: "in",
        html: '<ul><li data-each="items" style="color:{{.c}}">{{.name}}</li></ul>',
        fields: { items: rows({ c: "red" }, { c: "blue" }) },
      }),
      document.createElement("div"),
    );
    l.patch({ items: rows({ c: "red" }, { c: "green" }) });
    const r1 = lis(l)[1];
    expect(r1.style.color).toBe("green");
    expect(r1.style.getPropertyValue("--i")).toBe("1");
  });

  it("layer fields used inside rows update every row", () => {
    const l = new Layer(def({ state: "in" }), document.createElement("div"));
    const before = lis(l);
    l.patch({ title: "Final" });
    expect(lis(l)).toEqual(before);
    expect(texts(l)).toEqual(["1 Ada 3 Final", "2 Grace 2 Final"]);
    expect(l.shadow.querySelector("h1")!.textContent).toBe("Final");
  });

  it("off air: added and removed rows change at once, no animation bookkeeping", () => {
    fakeDuration("0.5s");
    vi.useFakeTimers();
    const l = new Layer(def(), document.createElement("div"));
    l.patch({ items: rows({ name: "A" }, { name: "B" }, { name: "C" }) });
    expect(texts(l).map((t) => t!.split(" ")[1])).toEqual(["A", "B", "C"]);
    l.patch({ items: rows({ name: "A" }) });
    expect(lis(l).length).toBe(1);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("on air: an added row enters with data-row=in and existing rows are not re-animated", () => {
    const l = new Layer(def({ state: "in" }), document.createElement("div"));
    const [r0, r1] = lis(l);
    const spies = [vi.spyOn(r0, "setAttribute"), vi.spyOn(r1, "setAttribute")];
    const rootSet = vi.spyOn(l.root!, "setAttribute");
    l.patch({ items: rows({ name: "Ada", score: "3" }, { name: "Grace", score: "2" }, { name: "Linus", score: "1" }) });
    const now = lis(l);
    expect(now.slice(0, 2)).toEqual([r0, r1]);
    expect(now[2].getAttribute("data-row")).toBe("in");
    expect(now[2].style.getPropertyValue("--i")).toBe("2");
    expect(now[2].textContent).toBe("3 Linus 1 Top");
    for (const s of [...spies, rootSet]) expect(s).not.toHaveBeenCalled();
  });

  it("on air: a removed row plays data-row=out and is detached on animationend", () => {
    fakeDuration("0.5s");
    vi.useFakeTimers();
    const l = new Layer(def({ state: "in" }), document.createElement("div"));
    const [r0, r1] = lis(l);
    l.patch({ items: rows({ name: "Ada", score: "3" }) });
    expect(l.lists[0].rows.length).toBe(1);
    expect(r1.isConnected || r1.parentNode).toBeTruthy();
    expect(r1.getAttribute("data-row")).toBe("out");
    expect(r0.getAttribute("data-row")).toBe("in");
    r1.dispatchEvent(new Event("animationend"));
    expect(r1.parentNode).toBeNull();
    expect(lis(l)).toEqual([r0]);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("on air: a removed row is detached by the fallback timer (duration + 50 ms, 3 s cap)", () => {
    fakeDuration("10s");
    vi.useFakeTimers();
    const l = new Layer(def({ state: "in" }), document.createElement("div"));
    const r1 = lis(l)[1];
    l.patch({ items: rows({ name: "Ada", score: "3" }) });
    vi.advanceTimersByTime(2999);
    expect(r1.parentNode).not.toBeNull();
    vi.advanceTimersByTime(1);
    expect(r1.parentNode).toBeNull();
  });

  it("uses getAnimations().finished for leaving rows when supported", async () => {
    vi.useFakeTimers();
    const l = new Layer(def({ state: "in" }), document.createElement("div"));
    const r1 = lis(l)[1];
    let finish!: () => void;
    const finished = new Promise<void>((r) => (finish = r));
    r1.getAnimations = () => [{ finished } as unknown as Animation];
    l.patch({ items: rows({ name: "Ada", score: "3" }) });
    expect(r1.getAttribute("data-row")).toBe("out");
    expect(r1.parentNode).not.toBeNull();
    finish();
    await finished;
    await Promise.resolve();
    expect(r1.parentNode).toBeNull();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("leaving rows are dropped at once when the layer is taken off the DOM", () => {
    fakeDuration("0.5s");
    vi.useFakeTimers();
    const stage = document.createElement("div");
    const l = new Layer(def({ state: "in" }), stage);
    const r1 = lis(l)[1];
    l.patch({ items: rows({ name: "Ada", score: "3" }) });
    l.cut();
    expect(r1.parentNode).toBeNull();
    expect(vi.getTimerCount()).toBe(0);
    l.setState("in");
    expect(lis(l).length).toBe(1);
  });

  it("rows present at take-in carry data-row=in", () => {
    const stage = document.createElement("div");
    const l = new Layer(def(), stage);
    l.setState("in");
    expect(lis(l).map((li) => li.getAttribute("data-row"))).toEqual(["in", "in"]);
  });

  it("a 100-row update patches in place within budget", () => {
    const make = (n: number, tag: string) =>
      rows(...Array.from({ length: n }, (_, i) => ({ name: `${tag}${i}`, score: String(i * 7) })));
    const l = new Layer(def({ state: "in", fields: { title: "T", items: make(100, "a") } }), document.createElement("div"));
    const nodes = lis(l);
    expect(nodes.length).toBe(100);
    // Warm up, then time a full 100-row change (every row's name changes).
    l.patch({ items: make(100, "b") });
    const runs: number[] = [];
    for (let k = 0; k < 10; k++) {
      const t = performance.now();
      l.patch({ items: make(100, k % 2 ? "c" : "d") });
      runs.push(performance.now() - t);
    }
    runs.sort((a, b) => a - b);
    const median = runs[5];
    console.log(`renderer: 100-row list update median ${median.toFixed(2)} ms (jsdom)`);
    expect(lis(l)).toEqual(nodes);
    expect(median).toBeLessThan(20); // jsdom is several times slower than Chromium; target there is 4 ms
  });
});

describe("lists through App", () => {
  const screen: ScreenInfo = { id: "s", name: "S", width: 1920, height: 1080, background: "transparent" };

  async function watch(l: Layer, f: () => void) {
    const records: MutationRecord[] = [];
    const mo = new MutationObserver((r) => records.push(...r));
    mo.observe(l.shadow, { subtree: true, attributes: true, childList: true, characterData: true });
    f();
    await Promise.resolve();
    mo.disconnect();
    return [...records, ...mo.takeRecords()];
  }

  it("a same-rev snapshot and layer message keep the rows untouched", async () => {
    const app = new App(new Stage(document.body));
    app.handle({ t: "snapshot", screen, layers: [def({ state: "in" })] });
    const l = app.layers.get("lb")!;
    const before = lis(l);
    expect(await watch(l, () => app.handle({ t: "snapshot", screen, layers: [def({ state: "in" })] }))).toEqual([]);
    expect(await watch(l, () => app.handle({ t: "layer", layer: def({ state: "in", z: 4 }) }))).toEqual([]);
    expect(lis(l)).toEqual(before);
  });

  it("a fields message patches rows; a snapshot with changed rows patches in place", () => {
    const app = new App(new Stage(document.body));
    app.handle({ t: "snapshot", screen, layers: [def({ state: "in" })] });
    const l = app.layers.get("lb")!;
    const [r0] = lis(l);
    app.handle({ t: "fields", layerId: "lb", fields: { items: rows({ name: "Ada", score: "4" }) } });
    expect(lis(l)).toEqual([r0]);
    expect(texts(l)).toEqual(["1 Ada 4 Top"]);
    const f = def().fields;
    app.handle({ t: "snapshot", screen, layers: [def({ state: "in", fields: f })] });
    expect(lis(l)[0]).toBe(r0);
    expect(texts(l)).toEqual(["1 Ada 3 Top", "2 Grace 2 Top"]);
  });
});

describe("row marks", () => {
  const items = (...r: Record<string, string>[]) => ({ title: "T", items: rows(...r) });
  const types: Record<string, FieldType> = { title: "text", items: "list" };

  it("turns reserved _keys into data-* attributes, validated", () => {
    const { div } = render(
      HTML,
      items(
        { name: "A", score: "1", _status: "now", _current: "true" },
        { name: "B", score: "2", _status: "Bad Value!", _row: "out", _state: "in", _x: "<script>" },
      ),
      types,
    );
    const [a, b] = [...div.querySelectorAll("li.row")];
    expect(a.getAttribute("data-status")).toBe("now");
    expect(a.getAttribute("data-current")).toBe("true");
    expect(b.hasAttribute("data-status")).toBe(false); // not a safe token
    expect(b.getAttribute("data-row")).toBe("in"); // renderer-owned names can't be set
    expect(b.hasAttribute("data-state")).toBe(false);
    expect(b.hasAttribute("data-x")).toBe(false);
  });

  it("moves and removes marks in place, without touching unchanged rows", () => {
    const fields = items({ name: "A", score: "1", _current: "true" }, { name: "B", score: "2" });
    const c = compile(HTML, types, (k) => fields[k as keyof typeof fields] ?? "");
    const div = document.createElement("div");
    div.append(c.frag);
    const [a, b] = [...div.querySelectorAll("li.row")];
    const spy = vi.spyOn(b, "setAttribute");
    fields.items = rows({ name: "A", score: "1" }, { name: "B", score: "2", _current: "true" });
    c.lists.forEach((l) => l.update());
    expect(a.hasAttribute("data-current")).toBe(false);
    expect(b.getAttribute("data-current")).toBe("true");
    expect(div.querySelectorAll("li.row")[0]).toBe(a); // same nodes
    spy.mockClear();
    c.lists.forEach((l) => l.update());
    expect(spy).not.toHaveBeenCalled(); // unchanged: no writes
  });
});

describe("images", () => {
  it("decode asynchronously unless the template says otherwise, also inside rows", () => {
    const { div } = render(
      '<div><img class="a" src="/assets/a.png"><img class="b" decoding="sync" src="/assets/b.png"><ul><li data-each="items"><img class="c" src="{{.logo}}"></li></ul></div>',
      { items: rows({ logo: "/assets/c.png" }) },
      { items: "list" },
    );
    expect(div.querySelector(".a")!.getAttribute("decoding")).toBe("async");
    expect(div.querySelector(".b")!.getAttribute("decoding")).toBe("sync");
    expect(div.querySelector(".c")!.getAttribute("decoding")).toBe("async");
  });
});

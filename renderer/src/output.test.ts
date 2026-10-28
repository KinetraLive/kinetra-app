import { afterEach, describe, expect, it, vi } from "vitest";
import type { ScreenInfo, ServerMsg, WireLayer } from "./protocol";
import { CHECKER, fitBox, KEY_FILTER, output, outputApps, OutputStage, parseOutput, regions } from "./output";

const screen: ScreenInfo = { id: "s", name: "S", width: 1920, height: 1080, background: "transparent" };

function layer(over: Partial<WireLayer> = {}): WireLayer {
  return {
    id: "l1",
    z: 1,
    templateRev: 1,
    html: '<div class="lt"><span class="name">{{name}}</span></div>',
    css: "",
    fields: { name: "Ada" },
    fieldTypes: { name: "text" },
    state: "out",
    ...over,
  };
}

const q = (s: string) => new URLSearchParams(s);

function viewport(w: number, h: number) {
  vi.stubGlobal("innerWidth", w);
  vi.stubGlobal("innerHeight", h);
}

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.replaceChildren();
  document.documentElement.removeAttribute("style");
  document.body.removeAttribute("style");
});

describe("parseOutput", () => {
  it("reads bg, mode and dual", () => {
    expect(parseOutput(q("output=o1&bg=00B140&mode=fill"))).toEqual({ bg: "#00b140", key: false, dual: "" });
    expect(parseOutput(q("output=o1&bg=000000&mode=key"))).toEqual({ bg: "#000000", key: true, dual: "" });
    expect(parseOutput(q("output=o1&bg=123abc&mode=fill&dual=h"))).toEqual({ bg: "#123abc", key: false, dual: "h" });
    expect(parseOutput(q("output=o1&dual=v")).dual).toBe("v");
    expect(parseOutput(q("output=o1&dual=hr")).dual).toBe("hr");
    expect(parseOutput(q("output=o1&dual=vr")).dual).toBe("vr");
  });

  it("falls back to black, fill and one region", () => {
    for (const bad of ["", "bg=red", "bg=%23000000", "bg=12345", "bg=1234567", "bg=url(x)"])
      expect(parseOutput(q(`output=o&${bad}`)).bg).toBe("#000000");
    expect(parseOutput(q("output=o&mode=KEY")).key).toBe(false);
    for (const bad of ["x", "H", "r", "hv", "hrr", "rh"]) expect(parseOutput(q(`output=o&dual=${bad}`)).dual).toBe("");
  });
});

describe("regions and fit", () => {
  it("one region, or two equal halves", () => {
    expect(regions(1920, 1080, "")).toEqual([[0, 0, 1920, 1080]]);
    expect(regions(3840, 1080, "h")).toEqual([
      [0, 0, 1920, 1080],
      [1920, 0, 1920, 1080],
    ]);
    expect(regions(1920, 2160, "v")).toEqual([
      [0, 0, 1920, 1080],
      [0, 1080, 1920, 1080],
    ]);
  });

  it("reversed: the key's half first on screen, still [fill, key]", () => {
    expect(regions(3840, 1080, "hr")).toEqual([
      [1920, 0, 1920, 1080],
      [0, 0, 1920, 1080],
    ]);
    expect(regions(1920, 2160, "vr")).toEqual([
      [0, 1080, 1920, 1080],
      [0, 0, 1920, 1080],
    ]);
    // An odd size: the halves still cover the viewport exactly.
    expect(regions(3841, 1080, "hr")).toEqual([
      [1921, 0, 1920, 1080],
      [0, 0, 1921, 1080],
    ]);
  });

  it("scales uniformly, centered (letterbox / pillarbox)", () => {
    expect(fitBox(1920, 1080, 1920, 1080)).toEqual({ k: 1, x: 0, y: 0 });
    expect(fitBox(3840, 2160, 1920, 1080)).toEqual({ k: 2, x: 0, y: 0 });
    // 16:9 screen on a 4:3 display: full width, bars top and bottom.
    expect(fitBox(1024, 768, 1920, 1080)).toEqual({ k: 1024 / 1920, x: 0, y: (768 - 576) / 2 });
    // 4:3 screen on a 16:9 display: full height, bars left and right.
    expect(fitBox(1920, 1080, 1440, 1080)).toEqual({ k: 1, x: 240, y: 0 });
    expect(fitBox(1920, 1080, 0, 0).k).toBe(0);
  });
});

describe("OutputStage", () => {
  it("fill: the region in the background color, the box fitted, no filter", () => {
    viewport(1024, 768);
    const st = new OutputStage(parseOutput(q("output=o&bg=00b140")), 0);
    st.apply(screen);
    expect(st.el.style.background).toBe("rgb(0, 177, 64)");
    expect(st.box.style.filter).toBe("");
    expect(st.el.style.width).toBe("1024px");
    expect(st.box.style.transform).toBe(`translate(0px,96px) scale(${1024 / 1920})`);
  });

  it("checker (the remote preview's capture): dark squares behind transparency", () => {
    viewport(640, 360);
    const o = parseOutput(q("output=preview&bg=checker&mute=1"));
    expect(o).toEqual({ bg: CHECKER, key: false, dual: "" });
    const st = new OutputStage(o, 0);
    st.apply(screen);
    // jsdom drops the conic gradient itself (Edge draws it); the square size stays.
    expect(st.el.style.backgroundSize).toBe("32px 32px");
    expect(st.box.style.transform).toBe(`translate(0px,0px) scale(${640 / 1920})`);
  });

  it("key: black with the matte filter on the screen box", () => {
    viewport(1920, 1080);
    const st = new OutputStage(parseOutput(q("output=o&bg=00b140&mode=key")), 0);
    expect(st.el.style.background).toBe("rgb(0, 0, 0)");
    expect(st.box.style.filter).toBe(KEY_FILTER);
  });

  it("refits on resize only", () => {
    viewport(1920, 1080);
    const st = new OutputStage(parseOutput(q("output=o")), 0);
    st.apply(screen);
    expect(st.box.style.transform).toBe("translate(0px,0px) scale(1)");
    viewport(960, 1080);
    expect(st.box.style.transform).toBe("translate(0px,0px) scale(1)");
    dispatchEvent(new Event("resize"));
    expect(st.box.style.transform).toBe("translate(0px,270px) scale(0.5)");
  });
});

describe("output windows", () => {
  it("hides the cursor and sets the page background", () => {
    viewport(1920, 1080);
    outputApps(parseOutput(q("output=o&bg=123456")));
    expect(document.documentElement.style.cursor).toBe("none");
    expect(document.body.style.cursor).toBe("none");
    expect(document.documentElement.style.background).toBe("rgb(18, 52, 86)");
  });

  it("dual h: fill on the left, key on the right", () => {
    viewport(3840, 1080);
    const [fill, key] = outputApps(parseOutput(q("output=o&bg=00b140&dual=h")));
    const f = fill.stage as OutputStage;
    const k = key.stage as OutputStage;
    expect([f.el.style.left, f.el.style.width, k.el.style.left, k.el.style.width]).toEqual([
      "0px",
      "1920px",
      "1920px",
      "1920px",
    ]);
    expect(f.el.style.background).toBe("rgb(0, 177, 64)");
    expect(f.box.style.filter).toBe("");
    expect(k.el.style.background).toBe("rgb(0, 0, 0)");
    expect(k.box.style.filter).toBe(KEY_FILTER);
  });

  it("dual hr / vr: key on the left / top, fill on the right / bottom", () => {
    viewport(3840, 1080);
    let [fill, key] = outputApps(parseOutput(q("output=o&bg=00b140&dual=hr")));
    let f = fill.stage as OutputStage;
    let k = key.stage as OutputStage;
    expect([f.el.style.left, f.el.style.top, k.el.style.left, k.el.style.top]).toEqual(["1920px", "0px", "0px", "0px"]);
    expect(f.el.style.background).toBe("rgb(0, 177, 64)");
    expect(f.box.style.filter).toBe("");
    expect(k.box.style.filter).toBe(KEY_FILTER);
    document.body.replaceChildren();
    viewport(1920, 2160);
    [fill, key] = outputApps(parseOutput(q("output=o&dual=vr")));
    f = fill.stage as OutputStage;
    k = key.stage as OutputStage;
    expect([f.el.style.top, f.el.style.height, k.el.style.top, k.el.style.height]).toEqual([
      "1080px",
      "1080px",
      "0px",
      "1080px",
    ]);
    expect(k.box.style.filter).toBe(KEY_FILTER);
  });

  it("dual: one handler call applies every message to both stacks in the same task", () => {
    viewport(1920, 2160);
    const apps = outputApps(parseOutput(q("output=o&dual=v")));
    const handle = (m: ServerMsg) => {
      for (const a of apps) a.handle(m);
    };
    const msgs: ServerMsg[] = [
      { t: "snapshot", screen, layers: [layer()] },
      { t: "state", layerId: "l1", state: "in" },
      { t: "fields", layerId: "l1", fields: { name: "Grace" } },
    ];
    const roots = () =>
      apps.map((a) => {
        const l = a.layers.get("l1");
        return l && l.host.isConnected && a.stage.box.contains(l.host) ? l.root : null;
      });
    handle(msgs[0]);
    expect(apps.map((a) => a.layers.size)).toEqual([1, 1]);
    expect(roots()).toEqual([null, null]);
    // No await between the calls and the checks: both halves changed synchronously.
    handle(msgs[1]);
    expect(roots().map((r) => r?.getAttribute("data-state"))).toEqual(["in", "in"]);
    handle(msgs[2]);
    expect(roots().map((r) => r?.textContent)).toEqual(["Grace", "Grace"]);
  });

  it("output() fans every message out to both halves", () => {
    viewport(3840, 1080);
    const handle = output(q("output=o&dual=h"));
    handle({ t: "snapshot", screen, layers: [layer({ state: "in" })] });
    const hosts = [...document.querySelectorAll<HTMLElement>("body > div > div > div")];
    expect(hosts).toHaveLength(2);
    expect(hosts.map((h) => h.shadowRoot?.querySelector(".lt")?.getAttribute("data-state"))).toEqual(["in", "in"]);
  });
});

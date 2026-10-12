// Output mode (`?output=<id>`): a Kinetra display output window. A lazy chunk, loaded by main.ts
// only when the URL has `output`; browser sources never load it.
//
// - `bg=<rrggbb>`: the page behind the stage (fill), default black;
// - `mode=key`: a black page, every layer as its alpha matte (white on black; one compositor
//   filter on the screen box, so semi-transparent pixels become the matching grey);
// - `dual=h|v|hr|vr`: one window spanning two displays: the viewport is split into two equal
//   halves (h = side by side, v = stacked), the first shows the fill, the second the key; `r`
//   reverses them (the key's display is left of / above the fill's).
//
// The stage is scaled uniformly to fit its region (centered, letterboxed in the background color),
// recomputed on `resize` only. The cursor is hidden. Dual mode renders the layer stack twice from
// ONE WebSocket: every message is applied to both Apps synchronously in the same task, so both
// halves change in the same frame (animations that start in one task share the document
// timeline).
import { App } from "./app";
import type { ServerMsg } from "./protocol";
import { Stage } from "./stage";

export interface OutputOpts {
  /** Page background for fill, `#rrggbb`. */
  bg: string;
  /** The whole window is a key (alpha matte). */
  key: boolean;
  /** One window spanning two displays: fill in the first half, key in the second (`r`: key first). */
  dual: "" | "h" | "v" | "hr" | "vr";
}

/** Reads the output query; anything invalid falls back to black / fill / single. */
export function parseOutput(q: URLSearchParams): OutputOpts {
  const bg = q.get("bg") || "";
  const d = q.get("dual");
  return {
    bg: /^[0-9a-f]{6}$/i.test(bg) ? "#" + bg.toLowerCase() : "#000000",
    key: q.get("mode") === "key",
    dual: d === "h" || d === "v" || d === "hr" || d === "vr" ? d : "",
  };
}

/** [x, y, w, h] in CSS px. */
export type Region = [number, number, number, number];

/** The viewport, or its two halves when `dual`: always [fill, key] (`r`: the key's half is the
 *  left / top one). */
export function regions(vw: number, vh: number, dual: OutputOpts["dual"]): Region[] {
  if (!dual) return [[0, 0, vw, vh]];
  let r: Region[];
  if (dual[0] === "h") {
    const a = Math.round(vw / 2);
    r = [
      [0, 0, a, vh],
      [a, 0, vw - a, vh],
    ];
  } else {
    const a = Math.round(vh / 2);
    r = [
      [0, 0, vw, a],
      [0, a, vw, vh - a],
    ];
  }
  return dual[1] ? r.reverse() : r;
}

/** Uniform scale of a w×h box into rw×rh, centered: the scale and the box's offset. */
export function fitBox(rw: number, rh: number, w: number, h: number): { k: number; x: number; y: number } {
  const k = w > 0 && h > 0 ? Math.max(0, Math.min(rw / w, rh / h)) : 0;
  return { k, x: (rw - w * k) / 2, y: (rh - h * k) / 2 };
}

/** The matte: white with the original alpha. */
export const KEY_FILTER = "brightness(0) invert(1)";

/** A stage placed in one region of the viewport (its `el` is the region, in its background). */
export class OutputStage extends Stage {
  constructor(
    readonly o: OutputOpts,
    readonly i: number,
    parent: Element = document.body,
  ) {
    super(parent);
    const key = o.key || i > 0;
    this.el.style.background = key ? "#000000" : o.bg;
    if (key) this.box.style.filter = KEY_FILTER;
    this.fit();
  }

  /** Places the region, then fits the screen box into it. Runs on snapshot and `resize` only. */
  fit() {
    const [x, y, w, h] = regions(innerWidth, innerHeight, this.o.dual)[this.i] || [0, 0, 0, 0];
    const s = this.el.style;
    s.left = x + "px";
    s.top = y + "px";
    s.width = w + "px";
    s.height = h + "px";
    if (!this.w || !this.h) return;
    const f = fitBox(w, h, this.w, this.h);
    this.box.style.transform = `translate(${f.x}px,${f.y}px) scale(${f.k})`;
  }
}

/** The Apps of an output window: one, or two (fill, key) for `dual`. */
export function outputApps(o: OutputOpts, onUnknown: () => void = () => {}): App[] {
  const de = document.documentElement.style;
  de.cursor = document.body.style.cursor = "none";
  de.background = o.key || o.dual ? "#000000" : o.bg;
  return regions(innerWidth, innerHeight, o.dual).map((_, i) => new App(new OutputStage(o, i), onUnknown));
}

/** Sets the page up and returns the message handler: every message goes to every App, in order,
 *  synchronously (one task). */
export function output(q: URLSearchParams, onUnknown?: () => void): (m: ServerMsg) => void {
  const apps = outputApps(parseOutput(q), onUnknown);
  return (m) => {
    for (const a of apps) a.handle(m);
  };
}

// Template compile/bind, field patching and the in/out animation contract for one layer.
import { now, show, sync, timerRuns, watch, type Ticker } from "./clock";
import { FONT_CAP_MS, faces, familiesIn, familiesInCss, fontsReady, loadFonts, useFonts } from "./fonts";
import type { FieldType, Fields, LayerState, Rect, WireLayer } from "./protocol";

/** `data-fit` elements whose text changed; fitted at the end of the same patch (`flush`). */
const dirty = new Set<HTMLElement>();

/** Sets or clears `data-empty` (toggleAttribute with force: no mutation when unchanged). */
const mark = (el: Element, on: boolean) => el.toggleAttribute("data-empty", on);

/**
 * `data-fit`: from its CSS font size (the inline one cleared), shrinks `el`'s font size until its
 * content fits its box's width (no horizontal overflow) and its height is at most
 * `data-fit-lines` lines (default 1; half a line of tolerance for glyph overflow and padding),
 * down to `data-fit-min` × (default 0.6). The box's own height is not a limit: use the lines.
 * The width fit is strict: no overshoot, not even a fraction of a pixel (an `ellipsis` would show),
 * to the LayoutUnit (1/64 px).
 * Text too wide on one line is sized by proportion (2 layouts); otherwise a 5-step binary search
 * (at most 7 layouts). A detached box is left at the CSS size.
 */
export function fit(el: HTMLElement) {
  const st = el.style;
  st.fontSize = "";
  if (!el.isConnected) return;
  const cs = getComputedStyle(el);
  const fs = parseFloat(cs.fontSize);
  // The allowed height at the CSS size: lines × line height ("normal" taken as 1.2).
  const max = ((+el.dataset.fitLines! || 1) + 0.5) * (parseFloat(cs.lineHeight) || fs * 1.2);
  let w = 0;
  const ok = (k: number) => {
    st.fontSize = k < 1 ? fs * k + "px" : "";
    const cw = el.clientWidth;
    // scrollWidth is rounded: equal to clientWidth, the content may still overhang by < 1 px.
    return (w = el.scrollWidth) <= cw && el.scrollHeight <= max * k + 1 && (w < cw || inside(el, cs));
  };
  if (ok(1)) return;
  let lo = Math.min(1, +el.dataset.fitMin! || 0.6);
  // Width scales with the font size: the fit is at most clientWidth / scrollWidth (usually it);
  // 1 px under it absorbs scrollWidth's rounding.
  let hi = Math.min(1, (el.clientWidth - 1) / w);
  // Too wide even at the minimum: the minimum, no search.
  if (lo < hi) {
    if (hi < 1 && ok(hi)) return;
    for (let i = 5; i--; ) {
      const m = (lo + hi) / 2;
      if (ok(m)) lo = m;
      else hi = m;
    }
  }
  st.fontSize = fs * lo + "px";
}

/**
 * Sub-pixel check: the content's laid-out width (a Range over it, unclipped) is within `el`'s
 * content box. Scale-free (a transform scales both sides). Without layout data: true.
 */
function inside(el: HTMLElement, cs: CSSStyleDeclaration) {
  const r = document.createRange();
  r.selectNodeContents(el);
  const c = r.getBoundingClientRect?.().width;
  const e = el.getBoundingClientRect().width;
  const n = (v: string) => parseFloat(v) || 0;
  const pad = n(cs.paddingLeft) + n(cs.paddingRight) + n(cs.borderLeftWidth) + n(cs.borderRightWidth);
  let box = n(cs.width);
  if (cs.boxSizing == "border-box") box -= pad;
  // The content in layout px (client px × layout / client); a LayoutUnit (1/64 px) is the grain,
  // and the tolerance: client rects are snapped to it while computed sizes aren't, so with
  // fractional sizes (a layer box scaling the unit) a shrink-to-fit box can read up to one over.
  return !(c && e && box > 0) || (c * (box + pad)) / e <= box + 1 / 64;
}

/** Fits the `data-fit` elements whose text changed. */
function flush() {
  dirty.forEach(fit);
  dirty.clear();
}

export type Swap = WireLayer["swap"];

/** Crossfade length (ms) of a variant swap. */
export const FADE_MS = 300;

/**
 * Preview bus (`/screen/<id>/preview`): every layer is shown settled. Going in, it appears in its
 * end state (finite animations finished, endless ones run); going out, it is removed at once; no
 * variant swap animates; leaving list rows go at once.
 */
export let preview = false;
export const setPreview = (on: boolean) => {
  preview = on;
};

/** The screen's size (set by `Stage.apply`): the host's box without a rect, and `fit`'s design
 *  size without a template design. */
export const scr = { w: 0, h: 0 };

/** Jumps the finite animations/transitions just started under `r` to their end (a swapped-in look
 *  appears settled). Infinite ones (tickers, loops) keep running. */
function settleAnims(r: Element | null) {
  for (const a of r?.getAnimations?.({ subtree: true }) ?? [])
    try {
      if (a.effect?.getComputedTiming().iterations !== Infinity) a.finish();
    } catch {
      /* not finishable */
    }
}

/** The previous look during a swap: removed when the swap ends; `pend`: the new host waits (outIn). */
interface Ghost {
  el: HTMLElement;
  stop: () => void;
  pend: boolean;
}

/** Variable keys; anything else is ignored. Mirrors the server's validation. */
const VAR_KEY = /^[a-z][a-z0-9_-]{0,31}$/;

/** Splits "a {{ k }} b" into ["a ", "k", " b"]: odd indices are keys (`.col` and `#` in rows). */
const SPLIT = /\{\{\s*([\w.-]+|#)\s*\}\}/;
const URL_ATTRS = new Set(["src", "href", "srcset", "poster", "xlink:href", "action", "formaction"]);

type Update = () => void;
export type Binds = Map<string, Update[]>;

function okUrl(u: string): boolean {
  // Browsers ignore ASCII whitespace/control chars inside URLs ("java\tscript:"), so strip them first.
  const s = u.replace(/[\x00-\x20]/g, "").toLowerCase();
  const m = /^([a-z][a-z\d+.-]*):/.exec(s);
  return !m || m[1] === "http" || m[1] === "https" || s.startsWith("data:image/");
}

/** Returns `v` unless `name` is a URL attribute and `v` holds a disallowed scheme, then "". */
export function safeAttr(name: string, v: string): string {
  if (!URL_ATTRS.has(name)) return v;
  const urls = name === "srcset" ? v.split(",").map((c) => c.trim().split(/\s+/)[0]) : [v];
  return urls.every(okUrl) ? v : "";
}

/**
 * A value bound into a `style` attribute: colours, numbers, lengths, percentages and plain words
 * only. No ( ) ; : { } \ quotes / @ <, so it can neither call url()/image-set() nor leave its
 * declaration. Anything else becomes "".
 */
export const styleVal = (v: string) => (/^[#\w.%+\- ]{0,64}$/.test(v) ? v : "");

/** Attributes a value may never be bound into. */
const unsafeName = (n: string) => n.startsWith("on") || n === "srcdoc";

/** Parses markup for an `html` field and drops scripts, handlers and unsafe URLs. */
export function parseHtml(html: string): DocumentFragment {
  const t = document.createElement("template");
  t.innerHTML = html;
  for (const el of Array.from(t.content.querySelectorAll("*"))) {
    if (el.localName === "script") {
      el.remove();
      continue;
    }
    for (const a of Array.from(el.attributes)) {
      const n = a.name.toLowerCase();
      if (unsafeName(n) || safeAttr(n, a.value) !== a.value) el.removeAttributeNode(a);
    }
  }
  return t.content;
}

export interface Compiled {
  frag: DocumentFragment;
  /** The first element of the template: it carries data-state. */
  root: Element | null;
  binds: Binds;
  lists: List[];
}

function addBind(binds: Binds, k: string, f: Update) {
  let l = binds.get(k);
  if (!l) binds.set(k, (l = []));
  if (!l.includes(f)) l.push(f);
}

/**
 * Replaces `{{key}}` in text and attributes under `root` with bindings that read their value
 * through `get` (clock/timer values are formatted), registers them in `binds`, applies them once.
 */
function bind(root: Node, types: Record<string, FieldType>, get: (k: string) => string, binds: Binds) {
  const all: Update[] = [];
  const on = (k: string, f: Update) => {
    addBind(binds, k, f);
    if (!all.includes(f)) all.push(f);
  };
  const val = (k: string) => show(types[k], get(k));
  const w = document.createTreeWalker(root, 5 /* SHOW_ELEMENT | SHOW_TEXT */);
  const nodes: Node[] = [];
  while (w.nextNode()) nodes.push(w.currentNode);
  /** Element → the keys whose values it shows (its text, URL attributes): data-empty, data-fit. */
  const shows = new Map<Element, string[]>();
  const use = (el: Element | null, k: string) => el && (shows.get(el) || shows.set(el, []).get(el)!).push(k);

  for (const n of nodes) {
    if (n.nodeType === 3) {
      const parts = (n as Text).data.split(SPLIT);
      if (parts.length < 2) continue;
      for (let i = 1; i < parts.length; i += 2) use(n.parentElement, parts[i]);
      const out = document.createDocumentFragment();
      parts.forEach((p, i) => {
        if (!(i % 2)) {
          if (p) out.append(p);
        } else if (types[p] === "html") {
          const box = document.createElement("span");
          box.style.display = "contents";
          on(p, () => box.replaceChildren(parseHtml(get(p))));
          out.append(box);
        } else {
          const tn = document.createTextNode("");
          on(p, () => {
            const v = val(p);
            if (tn.data !== v) tn.data = v;
          });
          out.append(tn);
        }
      });
      (n as Text).replaceWith(out);
    } else {
      const el = n as Element;
      for (const a of Array.from(el.attributes)) {
        const parts = a.value.split(SPLIT);
        if (parts.length < 2) continue;
        const name = a.name.toLowerCase();
        if (unsafeName(name)) {
          console.warn(`renderer: field binding in "${a.name}" ignored`);
          continue;
        }
        const ns = a.namespaceURI;
        const qn = a.name;
        const set = () => {
          const css = name === "style";
          const v = safeAttr(name, parts.map((p, i) => (i % 2 ? (css ? styleVal(val(p)) : val(p)) : p)).join(""));
          if (el.getAttribute(qn) !== v) el.setAttributeNS(ns, qn, v);
        };
        for (let i = 1; i < parts.length; i += 2) {
          on(parts[i], set);
          if (URL_ATTRS.has(name)) use(el, parts[i]);
        }
      }
    }
  }
  // After the text/attribute updates of the same keys: empty marks, then fit (at the flush).
  for (const [el, ks] of shows) {
    const box = el.closest<HTMLElement>("[data-fit]");
    const f = () => {
      mark(el, ks.every((k) => !val(k).trim()));
      if (box) dirty.add(box);
    };
    for (const k of ks) on(k, f);
  }
  all.forEach((f) => f());
}

/**
 * Parses template html once and binds `{{key}}` in text and attributes. An element with
 * `data-each="<listKey>"` is a row template (see List). All bindings are applied once.
 * `live` says whether the layer is on air (rows animate in/out only then).
 */
export function compile(
  html: string,
  types: Record<string, FieldType>,
  get: (k: string) => string,
  live: () => boolean = () => false,
): Compiled {
  const t = document.createElement("template");
  t.innerHTML = html;
  const frag = t.content;
  // Images (sponsor logos, slides) decode off the main thread unless the template says otherwise.
  for (const img of Array.from(frag.querySelectorAll("img"))) if (!img.hasAttribute("decoding")) img.setAttribute("decoding", "async");
  const root = frag.firstElementChild;
  const binds: Binds = new Map();
  const lists: List[] = [];
  if (root?.hasAttribute("data-each")) {
    console.warn("renderer: the template root cannot be a list row");
    root.removeAttribute("data-each");
  }
  for (const el of Array.from(frag.querySelectorAll("[data-each]"))) {
    if (el.parentElement?.closest("[data-each]")) continue; // nested rows: not supported
    const key = el.getAttribute("data-each") || "";
    const start = document.createComment(key);
    el.replaceWith(start);
    const lim = parseInt(el.getAttribute("data-limit") || "", 10);
    const l = new List(key, el, start, lim >= 0 ? lim : Infinity, types, get, live);
    lists.push(l);
    addBind(binds, key, () => l.update());
    // Layer fields used inside the row: forward their updates to every row.
    const keys = new Set(el.outerHTML.split(SPLIT).filter((m, i) => i % 2 && m !== key && m !== "#" && m[0] !== "."));
    for (const k of keys) addBind(binds, k, () => l.field(k));
  }
  bind(frag, types, get, binds);
  lists.forEach((l) => l.update());
  return { frag, root, binds, lists };
}

/** Parses a list value: a JSON array of row objects with string cells; anything else → no rows. */
export function parseRows(v: string): Record<string, string>[] {
  let a: unknown;
  try {
    a = JSON.parse(v);
  } catch {
    return [];
  }
  return Array.isArray(a)
    ? a.map((r) => {
        const o: Record<string, string> = Object.create(null);
        if (r && typeof r === "object")
          for (const [k, c] of Object.entries(r)) o[k] = c == null || typeof c === "object" ? "" : String(c);
        return o;
      })
    : [];
}

/** Sets the row's `--i` unless it already has that value (no style write on unchanged rows). */
function index(el: Element, i: number) {
  const st = (el as HTMLElement).style;
  if (st && st.getPropertyValue("--i") !== String(i)) st.setProperty("--i", String(i));
}

const MARK_KEY = /^_[a-z][a-z0-9-]{0,30}$/;
const MARK_VAL = /^[a-z0-9-]{1,32}$/;
/** Attribute names the renderer owns; a row mark may never set them. */
const OWN = new Set(["row", "each", "limit", "state"]);

/**
 * Row marks: reserved `_name` keys of a row (set by the server, e.g. `_status`, `_current`)
 * become `data-name` attributes on the row element, for CSS. Only safe tokens; removed marks are
 * removed; unchanged ones are not written.
 */
function marks(el: Element, d: Record<string, string>, old?: Record<string, string>) {
  for (const k in old)
    if (MARK_KEY.test(k) && !(k in d) && !OWN.has(k.slice(1))) el.removeAttribute("data-" + k.slice(1));
  for (const k in d) {
    if (!MARK_KEY.test(k) || OWN.has(k.slice(1))) continue;
    const name = "data-" + k.slice(1);
    const v = d[k];
    if (!MARK_VAL.test(v)) el.removeAttribute(name);
    else if (el.getAttribute(name) !== v) el.setAttribute(name, v);
  }
}

interface Row {
  el: Element;
  d: Record<string, string>;
  b: Binds;
}

/**
 * One `data-each` block. The row template is cloned per row after an anchor comment, with `--i`
 * and `data-row="in"`. Updates patch rows in place by index; on air, removed rows get
 * `data-row="out"` and are detached when their animation ends (off air: at once).
 */
export class List {
  rows: Row[] = [];
  private leaving = new Map<Element, () => void>();
  /** The row count last written (`data-count`). */
  private n = -1;

  constructor(
    readonly key: string,
    private readonly tpl: Element,
    private readonly start: Comment,
    private readonly limit: number,
    private readonly types: Record<string, FieldType>,
    private readonly get: (k: string) => string,
    private readonly live: () => boolean,
  ) {}

  update() {
    const data = parseRows(this.get(this.key)).slice(0, this.limit);
    const rows = this.rows;
    data.forEach((d, i) => {
      const r = rows[i];
      if (!r) return rows.push(this.make(i, d));
      const old = r.d;
      r.d = d;
      for (const k of new Set([...Object.keys(old), ...Object.keys(d)]))
        if ((old[k] ?? "") !== (d[k] ?? "")) r.b.get("." + k)?.forEach((u) => u());
      index(r.el, i); // a bound style attribute on the row itself may have dropped --i
      marks(r.el, d, old);
    });
    for (const r of rows.splice(data.length)) this.drop(r.el);
    // The element holding the rows: empty without rows; `data-count` / `--n` = rows on screen
    // (leaving ones not counted), written only on a change.
    const p = this.start.parentElement;
    const n = data.length;
    if (p) {
      mark(p, !n);
      if (n != this.n) {
        p.setAttribute("data-count", (this.n = n) + "");
        (p as HTMLElement).style?.setProperty("--n", n + "");
      }
    }
  }

  /** A layer field used inside the rows changed (or a clock ticked). */
  field(k: string) {
    for (const r of this.rows) r.b.get(k)?.forEach((u) => u());
  }

  private make(i: number, d: Record<string, string>): Row {
    const el = this.tpl.cloneNode(true) as Element;
    const r: Row = { el, d, b: new Map() };
    const f = document.createDocumentFragment();
    f.append(el);
    bind(f, this.types, (k) => (k === "#" ? String(i + 1) : k[0] === "." ? (r.d[k.slice(1)] ?? "") : this.get(k)), r.b);
    index(el, i);
    marks(el, d);
    el.setAttribute("data-row", "in");
    (i ? this.rows[i - 1].el : this.start).after(el);
    return r;
  }

  private drop(el: Element) {
    if (!this.live() || preview) return el.remove();
    el.setAttribute("data-row", "out");
    this.leaving.set(
      el,
      afterAnim(el, () => {
        this.leaving.delete(el);
        el.remove();
      }),
    );
  }

  /** Removes leaving rows now (layer detached or recompiled). */
  flush() {
    for (const [el, stop] of this.leaving) {
      stop();
      el.remove();
    }
    this.leaving.clear();
  }
}

/** "0.3s, 200ms" → [300, 200]. */
export function parseTimes(s: string | null | undefined): number[] {
  return (s || "").split(",").map((x) => {
    const n = parseFloat(x);
    return isNaN(n) ? 0 : /ms\s*$/.test(x) ? n : n * 1000;
  });
}

function total(dur: string, delay: string): number {
  const d = parseTimes(dur);
  const dl = parseTimes(delay);
  return Math.max(0, ...d.map((v, i) => v + dl[i % dl.length]));
}

/** Longest computed animation/transition (duration + delay) over the subtree, in ms. */
export function outDuration(root: Element): number {
  let max = 0;
  for (const el of [root, ...Array.from(root.querySelectorAll("*"))]) {
    const cs = getComputedStyle(el);
    max = Math.max(max, total(cs.animationDuration, cs.animationDelay), total(cs.transitionDuration, cs.transitionDelay));
  }
  return max;
}

export const OUT_CAP_MS = 3000;

/**
 * Calls `done` once the animations just started on `r` (or its subtree) end: getAnimations()
 * .finished, else animationend/transitionend on `r`, with a fallback timer from the computed
 * durations (+50 ms, capped at OUT_CAP_MS). Nothing running → `done` at once. Returns a cancel.
 */
export function afterAnim(r: Element, done: () => void): () => void {
  const ms = outDuration(r);
  // getAnimations() flushes style, so it already sees the animations just started.
  const anims = typeof r.getAnimations === "function" ? r.getAnimations({ subtree: true }) : null;
  if (!ms && !anims?.length) {
    done();
    return () => {};
  }
  let over = false;
  const h = (e: Event) => {
    if (e.target === r) end();
  };
  const stop = () => {
    over = true;
    clearTimeout(timer);
    r.removeEventListener("animationend", h);
    r.removeEventListener("transitionend", h);
  };
  const end = () => {
    if (over) return;
    stop();
    done();
  };
  if (anims) {
    if (anims.length) Promise.all(anims.map((a) => a.finished)).then(end, end);
  } else {
    r.addEventListener("animationend", h);
    r.addEventListener("transitionend", h);
  }
  const timer = setTimeout(end, Math.min(ms + 50, OUT_CAP_MS));
  return stop;
}

/** One layer: a host element with an open shadow root, compiled once per templateRev. */
export class Layer implements Ticker {
  host!: HTMLElement;
  shadow!: ShadowRoot;
  /** Holds the template in the shadow root: `display:contents` (reflow), or the design box
   *  (fit). */
  inner!: HTMLElement;
  def!: WireLayer;
  fields: Fields = {};
  /** Applied template variables, as set on the host (`--<key>`). */
  vars = new Map<string, string>();
  root: Element | null = null;
  binds: Binds = new Map();
  lists: List[] = [];
  /** null = not in the DOM; "out" = animating out, removal pending. */
  state: LayerState | null = null;
  /** Bound clock/timer field keys. */
  private ticks: string[] = [];
  private stop: (() => void) | undefined;
  /** Cancels a play-in waiting for fonts. */
  private pend: (() => void) | undefined;
  protected readonly parent: Element;
  /** The applied box as "x,y,w,h"; "" = the full screen. */
  private box = "";
  /** The host's size (0 = the screen's). */
  private bw = 0;
  private bh = 0;
  /** The inner box's applied style. */
  private sk = "";
  private ghost: Ghost | undefined;
  /** The next play-in shows settled (no in-animation); cleared once shown. */
  calm: boolean;

  /** `calm`: the next play-in appears settled (a studio take or T-bar mounts layers this way). */
  constructor(def: WireLayer, parent: Element, calm = false) {
    this.parent = parent;
    this.calm = calm;
    this.fresh();
    this.setVars(def.vars || {}, true);
    this.setRect(def.rect);
    this.build(def);
    this.setState(def.state);
  }

  /** A new, empty host (box and vars unset); returns the previous one. */
  private fresh(): HTMLElement {
    const old = this.host;
    this.host = document.createElement("div");
    // The host is the layer's box; overflow stays visible so out-animations can leave it.
    this.host.style.cssText = "position:absolute;left:0;top:0;width:100%;height:100%";
    this.shadow = this.host.attachShadow({ mode: "open" });
    this.inner = document.createElement("div");
    this.box = this.sk = "";
    this.vars = new Map();
    return old;
  }

  protected build(def: WireLayer) {
    // A new inner box per compile: the shadow root's children change in one mutation.
    if (this.inner.parentNode) (this.inner = document.createElement("div")), (this.sk = "");
    this.head(def);
    this.fields = { ...def.fields };
    const style = document.createElement("style");
    style.textContent = def.css || "";
    useFonts(familiesInCss(def.css || ""));
    this.lists.forEach((l) => l.flush());
    const types = def.fieldTypes || {};
    const c = compile(def.html || "", types, (k) => String(this.fields[k] ?? ""), () => this.state === "in");
    if (!c.root && !def.media) console.warn(`renderer: layer "${def.id}" has no template root element`);
    this.binds = c.binds;
    this.lists = c.lists;
    this.root = c.root;
    this.ticks = [...c.binds.keys()].filter((k) => types[k] === "clock" || types[k] === "timer");
    this.inner.replaceChildren(c.frag);
    this.shadow.replaceChildren(style, this.inner);
    flush(); // detached: nothing to measure yet
    watch(this, this.ticks.length > 0);
    loadFonts(this.need()); // preload, so a take later has nothing to wait for
  }

  /** The definition, its z-order and the template's auto-out as `--auto-out-ms` (absent: none). */
  private head(def: WireLayer) {
    this.def = def;
    const st = this.host.style;
    st.zIndex = String(def.z);
    const ms = def.autoOutMs;
    if (ms! > 0) st.setProperty("--auto-out-ms", ms + "ms");
    else st.removeProperty("--auto-out-ms");
    if (this.size()) this.fitAll();
  }

  /**
   * Content sizing. `fit`: the template lays out in an inner box of the design size (the screen's
   * without a design), scaled uniformly to fit the host's box and centered; it is the template's
   * containing block. `reflow`: `display:contents`, the content fills the host. From the numbers
   * only (no layout reads). True when the style changed.
   * Opacity (< 100) goes on this inner box, never on the host (swaps and studio takes animate the
   * host's opacity): reflow then needs a real box, `position:absolute;inset:0` (the host's size).
   */
  private size() {
    const d = this.def;
    if (!d) return false;
    const W = this.bw || scr.w;
    const H = this.bh || scr.h;
    const w = d.design?.w || scr.w;
    const h = d.design?.h || scr.h;
    const s = Math.min(W / w, H / h);
    const o = d.opacity! < 100 ? ";opacity:" + d.opacity! / 100 : "";
    const k =
      d.sizing == "fit" && s > 0 && s < 1 / 0
        ? `position:absolute;width:${w}px;height:${h}px;transform-origin:0 0;transform:translate(${(W - w * s) / 2}px,${(H - h * s) / 2}px) scale(${s})` + o
        : o
          ? "position:absolute;inset:0" + o
          : "display:contents";
    if (k == this.sk) return false;
    this.inner.style.cssText = this.sk = k;
    return true;
  }

  /** The fonts (family × weight) this layer's CSS and variables use. */
  private need() {
    return faces(this.def.css || "", this.vars.values());
  }

  /** Fits every `data-fit` element (only while in the DOM). */
  private fitAll() {
    if (this.host.isConnected) this.shadow.querySelectorAll<HTMLElement>("[data-fit]").forEach(fit);
  }

  /** On air, sizes or the look changed (no wait for fonts): fit now, and again once fonts load. */
  private later() {
    this.fitAll();
    loadFonts(this.need()).then(() => this.fitAll());
  }

  /** Ticker: on air with a bound clock or a running timer. */
  needs(): boolean {
    const types = this.def.fieldTypes || {};
    return this.state === "in" && this.ticks.some((k) => types[k] === "clock" || timerRuns(String(this.fields[k] ?? ""), now()));
  }

  /** Ticker: re-renders clock/timer bindings while on air (only changed text is written). */
  tick() {
    if (this.state === "in") for (const k of this.ticks) this.binds.get(k)?.forEach((u) => u());
    flush();
  }

  /**
   * Applies a full definition (`layer` message or snapshot entry). `swap` (live `layer` messages
   * only): a new rev on an on-air layer changes looks with that transition on a new host.
   */
  apply(def: WireLayer, swap?: Swap) {
    if (preview) swap = undefined;
    const rev = def.templateRev !== this.def.templateRev;
    if (rev) this.settle(true);
    const old = this.root;
    const sw = rev && old && this.state === "in" && def.state === "in" && (swap === "crossfade" || swap === "outIn") ? swap : undefined;
    const prev = sw && this.fresh();
    this.setVars(def.vars || {}, true);
    this.setRect(def.rect);
    if (prev && old) {
      this.build(def);
      const g: Ghost = { el: prev, stop: () => {}, pend: sw === "outIn" };
      this.ghost = g;
      if (g.pend) {
        old.setAttribute("data-state", "out");
        g.stop = afterAnim(old, () => this.settle(true));
        return;
      }
      // The new look, settled. The old one stays until the new one's fonts are in (a fallback
      // font never shows); ended before that, the new look shows at once.
      const start = () => {
        this.root?.setAttribute("data-state", "in");
        prev.after(this.host);
        this.fitAll();
        settleAnims(this.root);
        sync();
      };
      g.stop = () => (this.unpend(), start());
      return this.wait(() => {
        start();
        g.stop = () => {};
        // On-air graphics always animate as designed: the OS reduced-motion setting is for the app UI.
        const fade = (el: HTMLElement, a: number) =>
          el.animate?.({ opacity: [a, 1 - a] }, { duration: FADE_MS, easing: "cubic-bezier(0.2,0,0,1)", fill: "forwards" });
        const fin = fade(this.host, 0);
        fade(prev, 1);
        const t = setTimeout(() => this.settle(true), FADE_MS + 20);
        g.stop = () => {
          clearTimeout(t);
          fin?.cancel();
        };
      });
    }
    if (rev) {
      const was = this.state;
      this.cancel();
      this.unpend();
      this.build(def);
      if (was === "in" && def.state === "in") {
        // Already on air (a take of staged content, a reconnect): the new look appears settled,
        // it never replays its in-animation.
        this.root?.setAttribute("data-state", "in");
        settleAnims(this.root);
        this.later();
        sync();
      } else {
        // Off air or leaving: the new look plays in like a first take (fonts, fit).
        this.detach();
        this.setState(def.state);
      }
      return;
    }
    this.head(def);
    this.patch(def.fields || {}, true);
    this.setState(def.state);
  }

  /** Updates only the bindings whose value changed. `replace` treats `f` as the full field set. */
  patch(f: Fields, replace = false) {
    const old = this.fields;
    const next = replace ? { ...f } : { ...old, ...f };
    this.fields = next;
    for (const k of new Set([...Object.keys(old), ...Object.keys(next)])) {
      if ((old[k] ?? "") !== (next[k] ?? "")) this.binds.get(k)?.forEach((u) => u());
    }
    flush(); // data-fit in the same task as the text change: no visible reflow
    sync();
  }

  /**
   * Sets changed variables as custom properties on the host. Never recompiles or touches
   * data-state. `replace` treats `v` as the full set: missing keys are removed.
   */
  setVars(v: Record<string, string>, replace = false) {
    const old = this.vars;
    const next = new Map(replace ? [] : old);
    for (const k of Object.keys(v)) if (VAR_KEY.test(k) && typeof v[k] === "string") next.set(k, v[k]);
    this.vars = next;
    const st = this.host.style;
    const fonts = new Set<string>();
    for (const k of old.keys()) if (!next.has(k)) st.removeProperty("--" + k);
    for (const [k, val] of next) {
      if (old.get(k) === val) continue;
      st.setProperty("--" + k, val);
      familiesIn(val, fonts);
    }
    useFonts(fonts);
    // Sizes or fonts may have changed: fit again (nothing to measure off the DOM).
    if (this.host.isConnected) this.later();
  }

  /**
   * Positions the host at `r` (screen px); absent = the full screen. Only the host's position and
   * size change: no recompile, no data-state change. A rect with a non-finite number is ignored.
   */
  setRect(r: Rect | null | undefined) {
    let k = "";
    if (r) {
      const n = [r.x, r.y, r.w, r.h];
      if (!n.every(Number.isFinite)) return;
      k = n.join();
    }
    if (k === this.box) return;
    this.box = k;
    const st = this.host.style;
    st.left = r ? r.x + "px" : "0";
    st.top = r ? r.y + "px" : "0";
    st.width = r ? (this.bw = Math.max(0, r.w)) + "px" : "100%";
    st.height = r ? (this.bh = Math.max(0, r.h)) + "px" : "100%";
    if (!r) this.bw = this.bh = 0;
    this.size();
    this.fitAll(); // a new box: fit once
  }

  /**
   * Ends a running swap now: the old host goes; after an outIn the new one shows (`show`, still on
   * air) without waiting.
   */
  private settle(show: boolean) {
    const g = this.ghost;
    if (!g) return;
    this.ghost = undefined;
    g.stop();
    g.el.remove();
    if (g.pend && show && this.state === "in") {
      this.state = null;
      this.setState("in");
    } else sync();
  }

  /**
   * `in`: plays in once the fonts the layer uses have loaded (at once when they have; at most
   * FONT_CAP_MS later), so a fallback font never shows on air. A running out-animation continues
   * meanwhile. Already on air (or waiting): nothing changes, data-state is never touched.
   */
  setState(s: LayerState) {
    if (s === "in") {
      if (this.state !== "in" && !this.pend) this.wait(() => this.show());
      return;
    }
    this.unpend();
    if (preview) return this.cut();
    if (this.state === "in") {
      this.settle(true);
      this.state = "out";
      const r = this.root;
      if (!r) return this.detach();
      r.setAttribute("data-state", "out");
      this.stop = afterAnim(r, () => this.detach());
    }
    sync();
  }

  /**
   * Runs `go` once the fonts this layer uses have loaded: at once when they have, at most
   * FONT_CAP_MS later otherwise. `unpend` cancels it (no timer left).
   */
  private wait(go: () => void) {
    const f = this.need();
    if (fontsReady(f)) return go();
    const run = () => this.pend == p && (this.unpend(), go());
    const t = setTimeout(run, FONT_CAP_MS);
    const p = (this.pend = () => clearTimeout(t));
    loadFonts(f).then(run);
  }

  /** On air now, in one task: in the DOM, clocks current, text fitted, then data-state="in". */
  protected show() {
    this.cancel();
    if (this.host.parentNode !== this.parent) this.parent.append(this.host);
    this.state = "in";
    this.tick(); // clocks went stale off air
    this.fitAll();
    this.root?.setAttribute("data-state", "in");
    if (this.calm || preview) settleAnims(this.root);
    this.calm = false;
    sync();
  }

  private cancel() {
    this.stop?.();
    this.stop = undefined;
  }

  private unpend() {
    this.pend?.();
    this.pend = undefined;
  }

  /** PANIC: off air now, without an out-animation. The template stays compiled, so the next
   *  `in` animates in normally. */
  cut() {
    this.unpend();
    this.root?.setAttribute("data-state", "out");
    this.detach();
  }

  /** Removes the host from the DOM now. Leaving list rows go with it. */
  detach() {
    this.settle(false);
    this.cancel();
    this.lists.forEach((l) => l.flush());
    this.host.remove();
    this.state = null;
    sync();
  }

  /** The layer is gone for good: off the DOM and out of the clock scheduler. */
  dispose() {
    this.unpend();
    this.detach();
    watch(this, false);
  }
}

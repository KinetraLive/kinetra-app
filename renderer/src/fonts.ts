// Curated template fonts: a layer that references one gets /fonts/<slug>.css linked in the document.
// @font-face only applies from document scope (Chromium ignores it inside a shadow root's styles).

/** [family name, slug] (the slug: the name in lower case, spaces as hyphens). Mirrored in TEMPLATE_FONTS in
 * src-tauri/src/core/model.rs and src/lib/types.ts. */
export const FONTS: ReadonlyArray<readonly [string, string]> = (
  "Barlow,Barlow Condensed,Oswald,Montserrat,IBM Plex Sans,IBM Plex Sans Condensed,Source Serif 4,Roboto Mono,IBM Plex Mono,Space Grotesk,Manrope,Archivo,Playfair Display,Jost,Instrument Sans,Plus Jakarta Sans,Outfit"
)
  .split(",")
  .map((n) => [n, n.toLowerCase().replace(/ /g, "-")]);

const BY_NAME = new Map(FONTS.map(([n, s]) => [n.toLowerCase(), s]));
const NAME = new Map(FONTS.map(([n, s]) => [s, n]));

/** Slugs of curated families named in a font-family list or `font` shorthand value. */
export function familiesIn(value: string, out = new Set<string>()): Set<string> {
  for (const part of value.split(",")) {
    // The last words of an item are its family: `700 56px/1 "Barlow"`, `var(--font, 'Oswald')`.
    const words = part.replace(/["'()]/g, " ").trim().toLowerCase().split(/\s+/);
    for (let i = 0; i < words.length; i++) {
      const s = BY_NAME.get(words.slice(i).join(" "));
      if (s) {
        out.add(s);
        break;
      }
    }
  }
  return out;
}

/** Slugs of curated families used in `font`/`font-family` declarations or custom-property values. */
export function familiesInCss(css: string, out = new Set<string>()): Set<string> {
  const re = /(?:^|[{;\s])(font-family|font|--[\w-]+)\s*:([^;{}]*)/gi;
  const text = css.replace(/\/\*[\s\S]*?\*\//g, "");
  for (let m; (m = re.exec(text)); ) familiesIn(m[2], out);
  return out;
}

/** Ensures one stylesheet link per family in document.head; links are never removed. */
export function useFonts(slugs: Iterable<string>) {
  for (const s of slugs) {
    if (document.head.querySelector(`link[data-font="${s}"]`)) continue;
    const l = document.createElement("link");
    l.rel = "stylesheet";
    l.href = `/fonts/${s}.css`;
    l.dataset.font = s;
    sheets.set(s, new Promise((r) => (l.onload = l.onerror = r)));
    document.head.append(l);
  }
}

/** Longest wait for fonts before a first play-in (ms). */
export const FONT_CAP_MS = 1500;

/** [slug, font descriptor for document.fonts.load]. */
export type Face = [string, string];

/** Slug → resolves when its stylesheet has loaded (or failed). */
const sheets = new Map<string, Promise<unknown>>();
/** Descriptor → its (single) load. */
let loads = new Map<string, Promise<unknown>>();
/** Descriptors loaded (or failed: nothing better will come). */
let done = new Set<string>();

/** Test hook: forget every load. */
export function resetFonts() {
  loads = new Map();
  done = new Set();
}

/**
 * Every curated family the CSS or variable values use, times every weight the CSS names (the
 * first number or `bold` of each font-weight / font declaration) plus 400; when a font-style /
 * font declaration says `italic`, the italic of each too.
 */
export function faces(css: string, vals: Iterable<string>): Face[] {
  const fams = familiesInCss(css);
  for (const v of vals) familiesIn(v, fams);
  const ws = new Set(["400"]);
  for (const [, w] of css.matchAll(/font(?:-weight)?\s*:[^;{}]*?\b([1-9]00|bold)\b/gi)) ws.add(w == "bold" ? "700" : w);
  const st = /font(?:-style)?\s*:[^;{}]*\bitalic\b/i.test(css) ? ["", "italic "] : [""];
  const out: Face[] = [];
  for (const s of fams) for (const i of st) for (const w of ws) out.push([s, `${i}${w} 16px "${NAME.get(s)}"`]);
  return out;
}

/** Loads a face once: its stylesheet first (the @font-face must be known), then the font file. */
function load([s, d]: Face): Promise<unknown> {
  const ok = () => done.add(d);
  let p = loads.get(d);
  if (!p) loads.set(d, (p = Promise.resolve(sheets.get(s)).then(() => document.fonts.load(d)).then(ok, ok)));
  return p;
}

/** All faces loaded (or no font loading API: nothing to wait for). */
export const fontsReady = (f: Face[]) => !document.fonts || f.every(([, d]) => done.has(d));

/** Loads the faces (no timers; each once). Resolves when all are done or failed. */
export const loadFonts = (f: Face[]) => Promise.all(f.map(load));

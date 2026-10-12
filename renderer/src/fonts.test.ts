import { afterEach, describe, expect, it } from "vitest";
import { SLUGS, SUBSETS, WEIGHTS, italicsOf, plan, weightsOf } from "../scripts/fonts.mjs";
import { App } from "./app";
import { FONTS, familiesIn, familiesInCss } from "./fonts";
import type { ScreenInfo, WireLayer } from "./protocol";
import { Stage } from "./stage";

const screen: ScreenInfo = { id: "s", name: "S", width: 1920, height: 1080, background: "transparent" };
const BARLOW = '"Barlow", system-ui, sans-serif';

function layer(id: string, over: Partial<WireLayer> = {}): WireLayer {
  return {
    id,
    z: 1,
    templateRev: 1,
    html: "<div>{{name}}</div>",
    css: "div{font-family:var(--font)}",
    fields: { name: "Ada" },
    fieldTypes: {},
    state: "in",
    vars: {},
    ...over,
  };
}

const links = () =>
  Array.from(document.head.querySelectorAll<HTMLLinkElement>('link[rel="stylesheet"]')).map((l) => l.getAttribute("href"));

afterEach(() => {
  document.body.replaceChildren();
  document.head.querySelectorAll("link[data-font]").forEach((n) => n.remove());
});

describe("font links", () => {
  it("a font var with the Barlow stack links /fonts/barlow.css once; a second Barlow layer adds none", () => {
    const app = new App(new Stage(document.body));
    app.handle({ t: "snapshot", screen, layers: [layer("a", { vars: { font: BARLOW } })] });
    expect(links()).toEqual(["/fonts/barlow.css"]);
    app.handle({ t: "layer", layer: layer("b", { vars: { font: BARLOW } }) });
    expect(links()).toEqual(["/fonts/barlow.css"]);
  });

  it("a layer referencing no curated font adds none", () => {
    const app = new App(new Stage(document.body));
    app.handle({ t: "layer", layer: layer("a", { css: "div{font-family:system-ui,Arial;color:red}", vars: { accent: "#fff" } }) });
    expect(links()).toEqual([]);
  });

  it("CSS naming Oswald in a font-family links oswald", () => {
    const app = new App(new Stage(document.body));
    app.handle({ t: "layer", layer: layer("a", { css: "h1{font-family: 'Oswald', sans-serif}" }) });
    expect(links()).toEqual(["/fonts/oswald.css"]);
  });

  it("a vars message switching the font links the new family", () => {
    const app = new App(new Stage(document.body));
    app.handle({ t: "layer", layer: layer("a", { vars: { font: BARLOW } }) });
    app.handle({ t: "vars", layerId: "a", vars: { font: '"Roboto Mono", ui-monospace, monospace' } });
    expect(links()).toEqual(["/fonts/barlow.css", "/fonts/roboto-mono.css"]);
  });

  it("matches whole family names in stacks, shorthands, fallbacks and custom properties", () => {
    const f = (v: string) => [...familiesIn(v)];
    expect(f('"Barlow Condensed", "Arial Narrow", sans-serif')).toEqual(["barlow-condensed"]);
    expect(f("IBM Plex Sans")).toEqual(["ibm-plex-sans"]);
    expect(f('700 56px/1.1 "IBM Plex Sans Condensed", sans-serif')).toEqual(["ibm-plex-sans-condensed"]);
    expect(f("var(--font, 'Source Serif 4')")).toEqual(["source-serif-4"]);
    expect(f("Interstate, Barlowish")).toEqual([]);
    const css = ":host{--font:\"Montserrat\", sans-serif} .a{content:'Jost'} /* font-family: Oswald */ .b{font:600 20px Barlow}";
    expect([...familiesInCss(css)].sort()).toEqual(["barlow", "montserrat"]);
  });
});

describe("fonts build script", () => {
  it("covers the same families as the renderer", () => {
    expect(SLUGS).toEqual(FONTS.map(([, s]) => s));
  });

  it("emits unicode-range faces for latin and latin-ext only, with relative woff2 URLs", () => {
    const families = plan();
    const barlow = families.find((f) => f.slug === "barlow")!;
    const faces = barlow.css.match(/@font-face/g) || [];
    expect(faces.length).toBe(WEIGHTS.length * SUBSETS.length);
    expect(barlow.css.match(/unicode-range: U\+[0-9A-F]/g)?.length).toBe(faces.length);
    expect(barlow.css).toContain("font-display: swap");
    expect(barlow.css).toContain("font-family: 'Barlow'");
    for (const f of families) {
      for (const x of f.files) expect(x.name).toMatch(new RegExp(`^${f.slug}-(latin|latin-ext)-\\d00-(normal|italic)\\.woff2$`));
      for (const u of f.css.match(/url\([^)]*\)/g) || []) expect(u).toMatch(new RegExp(`^url\\(\\./${f.slug}/[\\w-]+\\.woff2\\)$`));
    }
    // Per-family weights: Space Grotesk ships 500 too (Frame), the others the default set.
    const sg = families.find((f) => f.slug === "space-grotesk")!;
    expect(weightsOf("space-grotesk")).toEqual([400, 500, 600, 700]);
    expect(sg.files.map((x) => x.name)).toContain("space-grotesk-latin-500-normal.woff2");
    expect(sg.css.match(/@font-face/g)?.length).toBe(4 * SUBSETS.length);
    expect(weightsOf("barlow")).toEqual(WEIGHTS);
    expect(italicsOf("barlow")).toEqual([]);
    expect(families.find((f) => f.slug === "archivo")!.files.map((x) => x.name)).toContain("archivo-latin-800-normal.woff2");
    // Italic faces (Elegant, Editorial): their own @font-face with font-style: italic.
    const pf = families.find((f) => f.slug === "playfair-display")!;
    expect(italicsOf("playfair-display")).toEqual([400, 600]);
    expect(pf.files.map((x) => x.name)).toContain("playfair-display-latin-600-italic.woff2");
    expect(pf.css.match(/font-style: italic/g)?.length).toBe(2 * SUBSETS.length);
    expect(pf.css.match(/font-style: normal/g)?.length).toBe(3 * SUBSETS.length);
    const ss = families.find((f) => f.slug === "source-serif-4")!;
    expect(ss.css).toMatch(/font-style: italic;\n {2}font-display: swap;\n {2}font-weight: 400;\n {2}src: url\(\.\/source-serif-4\/source-serif-4-latin-400-italic\.woff2\)/);
    // All 17 families, latin + latin-ext: 1.85 MB of woff2 now; the cap leaves ~5%.
    const total = families.reduce((n, f) => n + f.files.reduce((m, x) => m + x.bytes, 0), 0);
    expect(total).toBeLessThan(1.94 * 1024 * 1024);
  });
});

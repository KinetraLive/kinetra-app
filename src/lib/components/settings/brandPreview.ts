// Preferences > Style & brand, the lazy part (StyleBrand imports it once it is mounted, i.e. once
// the section scrolled into view):
//   - what an unset brand value inherits: the project style's look of the built-in lower third
//     (its variant `vars`, else the variable's definition default);
//   - the small live preview: that lower third in the style, with the brand kit over it and its
//     sample data, drawn settled by the renderer's own compile into an inert shadow root (the
//     variant thumbnails' renderThumb; no iframe). Redraws are debounced; no timer at rest.

import { releaseThumb, renderThumb } from "$lib/components/templates/editor/thumbs";
import { loadPresetLook, sampleValues, type PresetLook } from "$lib/components/templates/gallery/presets";
import { brandVars } from "$lib/components/templates/gallery/preview";
import { TEMPLATE_FONTS, type BrandKit } from "$lib/types";
import type { BrandColor, BrandFont } from "./brand";

export const PREVIEW_PRESET = "lower-third";
/** The lower third's default look: drawn for a style it has none of (Classic), at default values. */
const FALLBACK_VARIANT = "frame";
/** Debounce of a redraw after a value changes (ms). */
export const PREVIEW_MS = 150;

/** Brand kit key → the shared library variable it sets. */
export const BRAND_VARS: Record<BrandColor | BrandFont, string> = {
  accent: "accent",
  secondary: "secondary",
  text: "text",
  plate: "plate",
  fontHeading: "font-heading",
  fontBody: "font-body",
};

export type Inherited = Partial<Record<BrandColor | BrandFont, string>>;

export interface StyleLook {
  style: string;
  look: PresetLook;
  /** The lower third's sample data. */
  values: Record<string, string>;
  inherited: Inherited;
}

/** What each brand value inherits from `look`: its variant value, else the definition default. */
export function inheritedBrand(look: PresetLook): Inherited {
  const out: Inherited = {};
  for (const [key, v] of Object.entries(BRAND_VARS) as [BrandColor | BrandFont, string][]) {
    const value = look.vars[v] ?? look.variables.find((d) => d.key === v)?.default;
    if (value) out[key] = value;
  }
  return out;
}

/** The built-in lower third's look for `style` (null when it isn't in the build). */
export async function loadStyleLook(style: string): Promise<StyleLook | null> {
  let look = style ? await loadPresetLook(PREVIEW_PRESET, style) : null;
  let variant = style;
  if (!look) {
    const base = await loadPresetLook(PREVIEW_PRESET, FALLBACK_VARIANT);
    if (!base) return null;
    look = { ...base, vars: {} };
    variant = FALLBACK_VARIANT;
  }
  const values = await sampleValues(PREVIEW_PRESET, variant, look.fields);
  return { style, look, values, inherited: inheritedBrand(look) };
}

export interface PreviewInput {
  /** Brand kit values shown (saved or being edited); unset keys show the style's. */
  brand: BrandKit;
  /** URL of the brand logo on the output server, or null (the sample logo shows). */
  logo: string | null;
  /** Output server base URL while it runs (template fonts load from it), else null. */
  fontBase: string | null;
}

/** Links a template font's stylesheet from the output server once (document scope: @font-face). */
function useFont(base: string, name: string) {
  const slug = TEMPLATE_FONTS.find((f) => f.name === name)?.slug;
  if (!slug || document.head.querySelector(`link[data-brand-font="${slug}"]`)) return;
  const l = document.createElement("link");
  l.rel = "stylesheet";
  l.href = `${base.replace(/\/+$/, "")}/fonts/${slug}.css`;
  l.dataset.brandFont = slug;
  document.head.append(l);
}

function draw(el: HTMLElement, s: StyleLook, input: PreviewInput) {
  const vars = { ...s.look.vars, ...brandVars(input.brand) };
  const values = input.logo ? { ...s.values, logo: input.logo } : s.values;
  if (input.fontBase) {
    for (const key of ["fontHeading", "fontBody"] as const) {
      const name = input.brand[key] ?? s.inherited[key];
      if (name) useFont(input.fontBase, name);
    }
  }
  renderThumb(el, { html: s.look.html, css: s.look.css, vars }, s.look.fields, s.look.variables, { values });
}

/**
 * The preview in `el`. `set` draws at once when the look changed (first draw, another style) and
 * otherwise `delay` ms after the last change; `destroy` frees it.
 */
export class BrandPreview {
  #el: HTMLElement;
  #delay: number;
  #look: StyleLook | null = null;
  #input: PreviewInput | null = null;
  #timer: ReturnType<typeof setTimeout> | undefined;

  constructor(el: HTMLElement, delay = PREVIEW_MS) {
    this.#el = el;
    this.#delay = delay;
  }

  set(look: StyleLook | null, input: PreviewInput) {
    this.#input = input;
    if (!look) return;
    if (look !== this.#look) {
      this.#look = look;
      this.#flush();
      return;
    }
    if (this.#timer !== undefined) clearTimeout(this.#timer);
    this.#timer = setTimeout(() => this.#flush(), this.#delay);
  }

  #flush() {
    if (this.#timer !== undefined) clearTimeout(this.#timer);
    this.#timer = undefined;
    if (this.#look && this.#input && this.#el.isConnected) draw(this.#el, this.#look, this.#input);
  }

  destroy() {
    if (this.#timer !== undefined) clearTimeout(this.#timer);
    this.#timer = undefined;
    this.#look = null;
    releaseThumb(this.#el);
  }
}

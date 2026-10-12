// Brand kit form helpers (Preferences > Style & brand). Plain TypeScript, unit-testable.

import { isColor } from "$lib/components/variables/color";
import { TEMPLATE_FONTS, type BrandKit } from "$lib/types";

export type BrandColor = "accent" | "secondary" | "text" | "plate";
export type BrandFont = "fontHeading" | "fontBody";
export type BrandKey = BrandColor | BrandFont | "logo";

export const BRAND_COLORS: { key: BrandColor; label: string }[] = [
  { key: "accent", label: "Accent" },
  { key: "secondary", label: "Secondary" },
  { key: "text", label: "Text" },
  { key: "plate", label: "Plate" },
];

export const BRAND_FONTS: { key: BrandFont; label: string }[] = [
  { key: "fontHeading", label: "Heading font" },
  { key: "fontBody", label: "Body font" },
];

const FONTS = TEMPLATE_FONTS.map((f) => ({ value: f.name, label: f.name }));

/**
 * Font select options: unset first, showing the font the style uses and the style's name
 * ("Plus Jakarta Sans · Inline"; "Style's font" until known), then the template fonts.
 */
export function fontOptions(inherited: string | undefined, styleName: string): { value: string; label: string }[] {
  const unset = inherited ? (styleName ? `${inherited} · ${styleName}` : inherited) : "Style's font";
  return [{ value: "", label: unset }, ...FONTS];
}

/** Problem with a typed colour ("" = unset is fine), or null. */
export function colorIssue(v: string): string | null {
  return v === "" || isColor(v) ? null : "Enter a color like #1a1a1a.";
}

export { isHex, isTransparent, normalizeColor, pickerHex } from "$lib/components/variables/color";

/** The brand kit with `key` set (or unset for "" / null); unset keys are left out. */
export function withValue(brand: BrandKit, key: BrandKey, value: string | null): BrandKit {
  const next: BrandKit = { ...brand };
  if (value === null || value === "") delete next[key];
  else next[key] = value;
  return clean(next);
}

/** Only the keys that hold a value. */
export function clean(brand: BrandKit | null | undefined): BrandKit {
  const out: BrandKit = {};
  if (!brand) return out;
  for (const [k, v] of Object.entries(brand) as [BrandKey, string | null | undefined][]) if (v) out[k] = v;
  return out;
}

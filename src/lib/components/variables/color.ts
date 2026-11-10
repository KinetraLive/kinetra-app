// Colour values of variables and the brand kit, as the forms check them (the server applies the
// same rules in core/vars.rs): `#rrggbb`, `#rrggbbaa` or `transparent` (any case, stored lowercase).

export const TRANSPARENT = "transparent";

const HEX = /^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$/;

/** `#rrggbb` or `#rrggbbaa`. */
export const isHex = (v: string): boolean => HEX.test(v);

/** The `transparent` keyword, ignoring case. */
export const isTransparent = (v: string | null | undefined): boolean => v?.toLowerCase() === TRANSPARENT;

/** A hex colour or `transparent`. */
export const isColor = (v: string): boolean => isHex(v) || isTransparent(v);

/** `transparent` in any case becomes lowercase; anything else is returned as is. */
export const normalizeColor = (v: string): string => (isTransparent(v) ? TRANSPARENT : v);

/** `#rrggbb` part of a colour for `<input type="color">` (black when unset, transparent or invalid). */
export const pickerHex = (v: string): string => (HEX.test(v) ? v.slice(0, 7).toLowerCase() : "#000000");

/**
 * A hex colour typed or pasted by hand: `#E50914`, `E50914`, `#F00` or `F00` (spaces around are
 * ignored) → `#rrggbb` (three digits doubled, letter case kept), or null when it isn't one.
 */
export function parseHexInput(text: string): string | null {
  const m = /^#?([0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/.exec(text.trim());
  if (!m) return null;
  return `#${m[1].length === 3 ? [...m[1]].map((c) => c + c).join("") : m[1]}`;
}

/** The alpha suffix (`aa` of `#rrggbbaa`), or "". */
export const alphaOf = (v: string): string => (/^#[0-9a-fA-F]{8}$/.test(v) ? v.slice(7) : "");

// Style library helpers shared by the gallery, Preferences > Style & brand, the palette commands
// and the apply flow (./apply.svelte.ts). Small; every user of it is a lazy chunk.

import type { PresetInfo, StyleInfo } from "$lib/types";

/** The look the older presets had before the library; listed when `list_styles` leaves it out. */
export const CLASSIC: StyleInfo = {
  id: "classic",
  name: "Classic",
  description: "The original Kinetra look.",
};

/** The styles in library order, with Classic last when the backend doesn't list it. */
export const withClassic = (list: readonly StyleInfo[]): StyleInfo[] =>
  list.some((s) => s.id === CLASSIC.id) ? [...list] : [...list, CLASSIC];

/**
 * The styles worth offering: library order with Classic, and only those some built-in preset
 * ships (a style whose looks aren't built yet would switch nothing). Without presets (not
 * listed yet, or failed) every style is offered.
 */
export function offered(list: readonly StyleInfo[], presets: readonly PresetInfo[]): StyleInfo[] {
  const all = withClassic(list);
  if (presets.length === 0) return all;
  return all.filter((s) => presets.some((p) => p.variants?.some((v) => v.id === s.id)));
}

/**
 * A page whose own working copy an applied style makes stale (the layout editor): `before` runs
 * ahead of an apply or undo (flush pending saves), `after` once it is done (reload).
 */
export interface StyleHook {
  before?: () => Promise<unknown>;
  after?: () => void;
}

export const styleHooks = new Set<StyleHook>();

export async function beforeStyle(): Promise<void> {
  await Promise.all([...styleHooks].map((h) => h.before?.()));
}

export function afterStyle(): void {
  for (const h of styleHooks) h.after?.();
}

/** Registers a hook from two functions; returns the function that removes it. */
export function styleHook(before: () => Promise<unknown>, after: () => void): () => void {
  const h: StyleHook = { before, after };
  styleHooks.add(h);
  return () => void styleHooks.delete(h);
}

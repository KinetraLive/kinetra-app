// Studio mode's take transitions: names, the duration range and the defaults. Plain data, shared by
// Control (toolbar, settings, the studio column, the palette) and Flows (the Take preview node).

import type { TakeTransition, TakeTransitionKind } from "$lib/types";

export const TAKE_KINDS: readonly { value: TakeTransitionKind; label: string }[] = [
  { value: "cut", label: "Cut" },
  { value: "crossfade", label: "Crossfade" },
  { value: "outIn", label: "Play out & in" },
];

/** Crossfade duration (ms), as the server accepts it. */
export const TAKE_MS = { min: 100, max: 5000 } as const;

/** A new crossfade's duration (ms); the project default lives in studio.svelte.ts (DEFAULT_STUDIO). */
export const DEFAULT_MS = 500;

export const kindLabel = (k: TakeTransitionKind): string => TAKE_KINDS.find((x) => x.value === k)?.label ?? k;

/** "Cut", "Crossfade 500 ms", "Play out & in". */
export function transitionText(t: TakeTransition): string {
  return t.kind === "crossfade" ? `Crossfade ${t.ms} ms` : kindLabel(t.kind);
}

export const clampMs = (ms: number): number => Math.min(TAKE_MS.max, Math.max(TAKE_MS.min, Math.round(ms)));

/** A Take preview action's transition picker: the project default (null) or a given kind. */
export const TAKE_PICK: { value: TakeTransitionKind | "default"; label: string }[] = [
  { value: "default", label: "Project default" },
  ...TAKE_KINDS,
];

export const pickOf = (t: TakeTransition | null): TakeTransitionKind | "default" => t?.kind ?? "default";

/** The transition for a picker value, keeping the duration of `prev`. */
export function transitionFor(v: string, prev: TakeTransition | null): TakeTransition | null {
  const kind = TAKE_KINDS.find((k) => k.value === v)?.value;
  return kind ? { kind, ms: prev?.ms ?? DEFAULT_MS } : null;
}

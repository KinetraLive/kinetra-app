// Rules for the New screen dialog. Plain TypeScript so they are unit-testable.

import { SCREEN_NAME_MAX, SCREEN_SIZE_RANGE, type Resolution } from "$lib/types";

export type PresetKey = "1080p" | "720p" | "2160p" | "vertical" | "custom";

export const RESOLUTION_PRESETS: ReadonlyArray<{ key: Exclude<PresetKey, "custom">; label: string } & Resolution> = [
  { key: "1080p", label: "Full HD", width: 1920, height: 1080 },
  { key: "720p", label: "HD", width: 1280, height: 720 },
  { key: "2160p", label: "4K UHD", width: 3840, height: 2160 },
  { key: "vertical", label: "Vertical", width: 1080, height: 1920 },
];

/** The preset matching a resolution, else "custom". */
export function presetFor(r: Resolution): PresetKey {
  return RESOLUTION_PRESETS.find((p) => p.width === r.width && p.height === r.height)?.key ?? "custom";
}

/** One side of a screen: a whole number within SCREEN_SIZE_RANGE. */
export function validSide(n: number): boolean {
  return Number.isInteger(n) && n >= SCREEN_SIZE_RANGE.min && n <= SCREEN_SIZE_RANGE.max;
}

/** #rrggbb or #rrggbbaa. */
export function validHex(s: string): boolean {
  return /^#[0-9a-f]{6}([0-9a-f]{2})?$/i.test(s.trim());
}

/** A valid screen name: 1–SCREEN_NAME_MAX characters after trimming. */
export function validScreenName(name: string): boolean {
  const n = name.trim().length;
  return n >= 1 && n <= SCREEN_NAME_MAX;
}

/** "Screen N" with the smallest N ≥ count + 1 that no existing screen uses. */
export function defaultScreenName(names: readonly string[]): string {
  const taken = new Set(names.map((n) => n.trim().toLowerCase()));
  let n = names.length + 1;
  while (taken.has(`screen ${n}`)) n++;
  return `Screen ${n}`;
}

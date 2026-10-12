// Display rules for the Screens list. Plain TypeScript so they are unit-testable.

import type { LiveScreenView, ScreenSummary, ServerStatus } from "$lib/types";

export type ScreenTally = "program" | "preview" | "idle";

export const TALLY_LABELS: Record<ScreenTally, string> = {
  program: "On air",
  preview: "Ready",
  idle: "Server stopped",
};

/** On air if any layer of the screen's live view is on air; otherwise ready while the server
 *  runs; else idle. A screen without a live view has nothing on air. */
export function tallyOf(view: LiveScreenView | undefined, status: ServerStatus | null): ScreenTally {
  if (view?.layers.some((l) => l.onAir)) return "program";
  return status?.state === "running" ? "preview" : "idle";
}

/** Screens sorted by name (case-insensitive), so grid positions stay stable while renaming. */
export function sortScreens(list: readonly ScreenSummary[]): ScreenSummary[] {
  return [...list].sort(
    (a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }) || a.id.localeCompare(b.id),
  );
}

/** "1920×1080". */
export const resolutionLabel = (s: { width: number; height: number }) => `${s.width}×${s.height}`;

/** "1 layer", "3 layers". */
export const layersLabel = (n: number) => `${n} ${n === 1 ? "layer" : "layers"}`;

/** Scale that fits a w×h screen inside a box (contain); 0 while the box is unmeasured. */
export function fitScale(boxW: number, boxH: number, w: number, h: number): number {
  if (boxW <= 0 || boxH <= 0 || w <= 0 || h <= 0) return 0;
  return Math.min(boxW / w, boxH / h);
}

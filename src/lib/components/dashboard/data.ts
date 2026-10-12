// Dashboard data: what the dashboard shows and how it decides between the empty project state and
// the screens grid, and which templates count as recent. Plain TypeScript so the rules are unit-testable.

import {
  TEST_SCREEN_ID,
  type LiveScreenView,
  type ServerStatus,
  type TemplateSummary,
} from "$lib/types";

/** Rows in the dashboard's "Recent templates" panel. */
export const RECENT_TEMPLATES = 5;

/** The most recently changed templates, newest first (by `updatedAt`), at most `limit`. */
export function recentTemplates(
  list: readonly TemplateSummary[],
  limit: number = RECENT_TEMPLATES,
): TemplateSummary[] {
  const time = (t: TemplateSummary) => {
    const ms = Date.parse(t.updatedAt);
    return Number.isNaN(ms) ? -Infinity : ms;
  };
  return [...list]
    .sort((a, b) => time(b) - time(a) || a.name.localeCompare(b.name))
    .slice(0, Math.max(0, limit));
}

/** Screens the user made: every live screen except the hidden test screen (used by the renderer
 *  tests). */
export function userScreens(all: readonly LiveScreenView[]): LiveScreenView[] {
  return all.filter((s) => s.screen.id !== TEST_SCREEN_ID);
}

/** The project has nothing to show yet: no screens and no templates. */
export function isEmptyProject(
  screens: readonly LiveScreenView[],
  templates: readonly TemplateSummary[],
): boolean {
  return screens.length === 0 && templates.length === 0;
}

export type ScreenTally = "program" | "preview" | "idle";

/** On air if any layer is on air; otherwise ready (preview) while the server runs; else idle. */
export function screenTally(view: LiveScreenView, status: ServerStatus | null): ScreenTally {
  if (view.layers.some((l) => l.onAir)) return "program";
  return status?.state === "running" ? "preview" : "idle";
}

export const TALLY_LABELS: Record<ScreenTally, string> = {
  program: "On air",
  preview: "Ready",
  idle: "Server stopped",
};

// The live-thumbnail cap lives with LiveThumbs; re-exported for the dashboard tests.
export { MAX_LIVE_THUMBNAILS, pickLive } from "$lib/components/live/pick";

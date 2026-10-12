// Template constants a route needs without loading the runtime half of $lib/types (fonts, limits).
// $lib/types re-exports PREVIEW_SCREEN_ID.

/** The reserved, unlisted screen the template list and editor preview into (core/live.rs). */
export const PREVIEW_SCREEN_ID = "__preview";

/**
 * Window event after a template took its preset's latest look, or that update was undone
 * (`detail`: the TemplateDoc as saved). Sent by the update flow (templates/update.ts); the
 * templates list reloads on it.
 */
export const TEMPLATE_UPDATED = "kinetra:template-updated";

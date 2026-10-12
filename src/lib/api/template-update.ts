// Updating a preset template to the preset's latest look, and its undo. Only the lazy update
// flow (templates/update.ts) loads this module.

import type { TemplateDoc, TemplateUpdated } from "../types";
import { call } from "./index";

/** A `pre-update` backup, then the preset's latest looks; screens using it follow the live rules. */
export const updateTemplateFromPreset = (id: string) =>
  call<TemplateUpdated>("update_template_from_preset", { id });
/** The template as backup `backup` holds it; screens using it follow. */
export const undoTemplateUpdate = (id: string, backup: string) =>
  call<TemplateDoc>("undo_template_update", { id, backup });

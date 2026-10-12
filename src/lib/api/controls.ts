// The project's MIDI mappings (`controls.json`). Split out of the core `$lib/api` so only the
// pieces that edit mappings load it. Changes from any path arrive as `controls-changed`
// (`onControlsChanged` in `$lib/api/midi`).

import type { ControlsDoc } from "../types";
import { call } from "./index";

/** The open project's mappings. */
export const getControls = () => call<ControlsDoc>("get_controls");
/** Validates and stores the mappings (written coalesced); returns what was stored. Conflicting
 *  mappings are allowed (they are shown, and neither runs). */
export const saveControls = (doc: ControlsDoc) => call<ControlsDoc>("save_controls", { doc });

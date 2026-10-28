// Remote control's button panel (`panel.json`). Split out of the core `$lib/api` so only the
// panel editor loads it.

import type { PanelDoc, PanelTargets, PanelView } from "../types";
import { call, on } from "./index";

/** The open project's buttons and the steps whose target is missing. */
export const getPanel = () => call<PanelView>("get_panel");
/** Validates and stores the buttons (written coalesced); returns what was stored. */
export const savePanel = (doc: PanelDoc) => call<PanelView>("save_panel", { doc });
/** What the pickers offer: screens with their layers (top first), cues, table sources' rows. */
export const panelTargets = () => call<PanelTargets>("panel_targets");
/** The panel after a save, an outside edit, a restore or a project switch. */
export const onPanelChanged = (h: (v: PanelView) => void) => on("panel-changed", h);

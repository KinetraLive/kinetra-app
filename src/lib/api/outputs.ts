// Display outputs: displays, output status, test patterns. Configs live in prefs (`prefs.outputs`,
// saved with the prefs store); the core reconciles windows on every prefs change.

import type { DisplayInfo, OutputStatus, TestPattern } from "../types";
import { call } from "./index";

/** The connected displays (enumerates now; cheap). */
export const listDisplays = () => call<DisplayInfo[]>("list_displays");
/** Counted: while ≥ 1 watcher, display changes are pushed as `displays-changed` even with no
 * output enabled (Preferences › Outputs). Returns the current list. */
export const watchDisplays = (on: boolean) => call<DisplayInfo[]>("watch_displays", { on });
/** Status of every configured output. */
export const listOutputs = () => call<OutputStatus[]>("list_outputs");
/** Shows a test pattern on an output (and its key pair), or the screen again with null. Turns an
 * off output on for the pattern only (it goes off again with null). */
export const outputTestPattern = (id: string, pattern: TestPattern | null) =>
  call<void>("output_test_pattern", { id, pattern });
/** Shows each display's name and resolution full-screen for 3 s. */
export const identifyDisplays = () => call<void>("identify_displays");
/** Turns every output off (Output › All outputs off). */
export const allOutputsOff = () => call<void>("all_outputs_off");

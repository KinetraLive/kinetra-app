// Automation: timers and source auto-advance (server-side; flows are in `$lib/api/flows`).
// Split out of the core `$lib/api` so only the routes that use these commands load them.

import type { AutoAdvance, LiveLayerView, SourceSummary, TimerAction } from "../types";
import { call } from "./index";

/** One state change of a layer's timer field (saved; on air at once). */
export const setTimer = (screenId: string, layerId: string, key: string, action: TimerAction) =>
  call<LiveLayerView>("set_timer", { screenId, layerId, key, action });
/** Auto-advance on (settings) or off (null). */
export const setAutoAdvance = (sourceId: string, auto: AutoAdvance | null) =>
  call<SourceSummary>("set_auto_advance", { sourceId, auto });

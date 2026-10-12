// Resetting settings (Settings > Reset).
// Split out of the core `$lib/api` so only the routes that use these commands load them.

import type { ResetScope } from "../types";
import { call } from "./index";

export const resetSettings = (scope: ResetScope) => call<void>("reset_settings", { scope });

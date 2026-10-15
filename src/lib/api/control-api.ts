// The local Control API (Bitfocus Companion, scripts): the switch, the token and the connected
// clients. Split out of the core `$lib/api`; only Preferences › Network & API loads it.

import type { ControlApiView } from "../types";
import { call, on } from "./index";

/** The switch and the connected clients. */
export const controlApiView = () => call<ControlApiView>("control_api_view");
/** On (a token is made the first time) or off (every client disconnects). */
export const controlApiSet = (on: boolean) => call<ControlApiView>("control_api_set", { on });
/** The token, for Copy (it lives in the keychain; never shown in full). */
export const controlApiToken = () => call<string>("control_api_token");
/** A new token; connected clients disconnect and need it. */
export const controlApiRegenerateToken = () => call<string>("control_api_regenerate_token");
/** Disconnects one client. */
export const controlApiDisconnect = (id: number) => call<void>("control_api_disconnect", { id });
/** The switch or the client list changed. */
export const onControlApi = (h: (v: ControlApiView) => void) => on("control-api", h);

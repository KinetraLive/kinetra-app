// Live data (WebSocket) sources: create, test connection, retry, pause, and "in use" while the
// editor shows one. A module of its own so only the live data editor, the New source dialog and
// the lazy status parts carry it.

import type { SocketTestResult, SourceDoc, SourceSummary } from "../types";
import { call } from "./index";

export const createSocketSource = (name: string) => call<SourceDoc>("create_socket_source", { name });
/** Connects with the draft (unsaved) settings for up to 10 s; nothing is saved. */
export const testSocketSource = (doc: SourceDoc) => call<SocketTestResult>("test_socket_source", { doc });
/** Connect now (clears a hold and the reconnect wait). */
export const retrySocketSource = (id: string) => call<SourceSummary>("retry_socket_source", { id });
export const setSocketPaused = (id: string, paused: boolean) => call<SourceSummary>("set_socket_paused", { id, paused });
/** The editor shows this source: it counts as "in use", so it connects (counted). */
export const watchSocketSource = (id: string, on: boolean) => call<void>("watch_socket_source", { id, on });

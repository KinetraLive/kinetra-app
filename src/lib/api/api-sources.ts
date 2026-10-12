// API sources: create, test request, keychain secrets, retry, pause, and "in use" while the
// editor shows one. Apart from ./sources.ts (which Control and the flow editor load at first paint)
// so only the API editor, the New source dialog and Control's lazy sources strip carry them.

import type { ApiTestResult, SourceDoc, SourceSummary } from "../types";
import { call } from "./index";

export const createApiSource = (name: string) => call<SourceDoc>("create_api_source", { name });
/** One fetch with the draft (unsaved) settings; nothing is saved. */
export const testApiSource = (doc: SourceDoc) => call<ApiTestResult>("test_api_source", { doc });
/** Stores a header's value in the OS keychain (write-only; never read back). Returns the doc with
 * the header's `secret` id set. */
export const setSourceSecret = (sourceId: string, header: string, value: string) =>
  call<SourceDoc>("set_source_secret", { sourceId, header, value });
export const clearSourceSecret = (sourceId: string, header: string) =>
  call<SourceDoc>("clear_source_secret", { sourceId, header });
export const retryApiSource = (id: string) => call<SourceSummary>("retry_api_source", { id });
export const setApiPaused = (id: string, paused: boolean) => call<SourceSummary>("set_api_paused", { id, paused });
/** The editor shows this source: it counts as "in use" for polling (counted). */
export const watchApiSource = (id: string, on: boolean) => call<void>("watch_api_source", { id, on });

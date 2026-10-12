// A namespace import, read at call time: the shell's command registry imports this store, and tests
// that mock $lib/api without the UI state commands must still load it.
import * as api from "$lib/api";
import type { UiState } from "$lib/types";
import { SyncedStore } from "./synced.svelte";

/**
 * Per-project UI state (`<project>/ui-state.json`). Loaded when a project opens and cleared when it
 * closes; `value` is null while no project is open.
 */
export const uiState = new SyncedStore<UiState>(
  () => api.getUiState(),
  (v) => api.setUiState(v),
);

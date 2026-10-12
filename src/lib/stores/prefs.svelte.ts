// A namespace import, read at call time: the shell's key bindings import this store, and tests
// that mock $lib/api without the prefs commands must still load it.
import * as api from "$lib/api";
import type { Prefs } from "$lib/types";
import { SyncedStore } from "./synced.svelte";

/** App-wide preferences (`prefs.json` in the app config dir). Loaded once at startup. */
export const prefs = new SyncedStore<Prefs>(
  () => api.getPrefs(),
  (v) => api.setPrefs(v),
);

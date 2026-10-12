import * as api from "$lib/api";
import type { Project, RecentProject } from "$lib/types";
import { uiState } from "./uiState.svelte";

// Re-exported for routes that need only uiState (/flows): importing it through here keeps both
// stores in one chunk, which the picker loads (a separate uiState chunk costs it ~0.1 KB gz).
export { uiState };

// The open project and the recents list. One project is open at a time; opening another closes the
// current one in the backend (which flushes its stores) before the new one loads.

class ProjectStore {
  current = $state.raw<Project | null>(null);
  recents = $state.raw<RecentProject[]>([]);
  /** True while an open/create/init/close is in flight, to disable double-clicks. */
  busy = $state(false);

  async #run(op: () => Promise<Project | null>): Promise<Project | null> {
    this.busy = true;
    try {
      const p = await op();
      this.current = p;
      if (p) await uiState.load();
      else uiState.set(null);
      return p;
    } finally {
      this.busy = false;
    }
  }

  /** Restores the backend's open project after a webview reload. */
  async refresh() {
    const p = await api.currentProject();
    this.current = p;
    if (p) await uiState.load();
    else uiState.set(null);
  }

  open(path: string) {
    return this.#run(() => api.openProject(path)) as Promise<Project>;
  }

  create(parent: string, name: string) {
    return this.#run(() => api.createProject(parent, name)) as Promise<Project>;
  }

  /** Initializes an empty folder, or a foreign one after the user opted in. */
  init(path: string, name?: string) {
    return this.#run(() => api.initProject(path, name)) as Promise<Project>;
  }

  close() {
    return this.#run(async () => {
      await api.closeProject();
      return null;
    });
  }

  /** Called by commands that return the project after changing it (settings, restore, move). */
  replace(p: Project) {
    this.current = p;
  }

  async loadRecents() {
    this.recents = await api.listRecents();
  }

  async removeRecent(path: string) {
    this.recents = await api.removeRecent(path);
  }

  /**
   * Keeps `recents` in sync with the backend's deferred existence check. Call from a component's
   * `$effect` and return the result so the listener is released on teardown.
   */
  watchRecents(): () => void {
    return api.on("recents-checked", (list) => {
      this.recents = list;
    });
  }
}

export const project = new ProjectStore();

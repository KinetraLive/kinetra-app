// The project's screens for the Flows pages (the editor; the /flows list's summaries and recipe
// gallery, lazily). Not the shared live store: a module both Control and these routes import is
// split out of Control's route chunk, which costs its budget.
//
//   const screens = new ScreenViews();
//   $effect(() => screens.connect());

import { on } from "$lib/api";
import { listScreens } from "$lib/api/live";
import { TEST_SCREEN_ID, type LiveScreenView } from "$lib/types";

/**
 * The project's screens with their layers and field definitions, for names in summaries and the
 * inspector's pickers: `list_screens`, then `live-changed` per screen and a reload when a screen
 * file is added or removed. (Not the shared live store: see above.)
 */
export class ScreenViews {
  map = $state.raw<ReadonlyMap<string, LiveScreenView>>(new Map());
  list = $derived([...this.map.values()]);

  #alive = false;

  /** Reloads the list (e.g. after a recipe created a screen). */
  load = (): Promise<void> =>
    listScreens().then(
      (all) => {
        if (this.#alive) this.map = new Map((all ?? []).filter((v) => !reserved(v.screen.id)).map((v) => [v.screen.id, v]));
      },
      () => {},
    );

  connect(): () => void {
    this.#alive = true;
    const offFiles = on("project-files-changed", (c) => {
      if (c.kind === "screen" && c.change !== "invalid") void this.load();
    });
    const offLive = on("live-changed", (v) => {
      if (reserved(v.screen.id)) return;
      this.map = new Map(this.map).set(v.screen.id, v);
    });
    void this.load();
    return () => {
      this.#alive = false;
      offLive();
      offFiles();
    };
  }
}

const reserved = (id: string) => id === TEST_SCREEN_ID || id.startsWith("__");

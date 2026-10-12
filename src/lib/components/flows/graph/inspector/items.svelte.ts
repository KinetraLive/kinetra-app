// The items of a schedule source as picker options ("09:30 Gates open"), loaded once per source
// per editor session for the "a chosen item" pickers.

import { getSchedule } from "$lib/api/sources";
import { hhmm } from "$lib/components/recipes/schedule";
import type { Opt } from "../../pickers";

class ScheduleItems {
  #map = $state.raw<Record<string, Opt[]>>({});

  /** The items of `sourceId` (empty until loaded; a failed load stays empty). */
  of(sourceId: string): Opt[] {
    if (!sourceId) return [];
    const have = this.#map[sourceId];
    if (have) return have;
    queueMicrotask(() => this.#load(sourceId));
    return [];
  }

  #load(id: string) {
    if (this.#map[id]) return;
    this.#map = { ...this.#map, [id]: [] };
    getSchedule(id).then(
      (doc) => {
        this.#map = {
          ...this.#map,
          [id]: doc.items.map((i) => ({ value: i.id, label: `${hhmm(i.start)} ${i.title || "Untitled"}` })),
        };
      },
      () => {},
    );
  }
}

export const scheduleItems = new ScheduleItems();

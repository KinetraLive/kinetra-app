// What the /flows rows' trigger summaries need, as one lazy chunk of the list: the summary code
// (linear.ts, node labels) and what it names (the screens, kept current; the sources). The page
// imports it while the list loads and renders the names first. The recipe gallery takes its
// screens from here too.
//
//   const s = new Summaries(() => flows.list);
//   $effect(() => s.connect());
//   s.text(flow)

import { listSources } from "$lib/api/sources";
import type { FlowInfo, SourceSummary } from "$lib/types";
import type { Lookups } from "../linear";
import { ScreenViews } from "./screens.svelte";
import { triggerSummary } from "./summary";

export class Summaries {
  readonly screens = new ScreenViews();
  #sources = $state.raw<SourceSummary[]>([]);
  #flows: () => readonly FlowInfo[] = () => [];

  constructor(flows: () => readonly FlowInfo[]) {
    this.#flows = flows;
  }

  readonly lookups: Lookups = $derived({
    screens: this.screens.map,
    sources: new Map(this.#sources.map((s) => [s.id, s])),
    flows: new Map(this.#flows().map((f) => [f.id, f])),
  });

  connect(): () => void {
    listSources().then(
      (s) => (this.#sources = s ?? []),
      () => {},
    );
    return this.screens.connect();
  }

  text = (f: FlowInfo): string => triggerSummary(f, this.lookups);
}

// Lazy chunks of Control's side panel, each loaded on first use and cached (a failed load is
// retried next time): the Automation strip and the sources strip.

import type { Component } from "svelte";
import type { AutoAdvance, SourceSummary } from "$lib/types";
import type { ControlCtl } from "./control.svelte";
import type MenuComponent from "./Menu.svelte";

/**
 * Control's Menu, handed down by the static shells rather than imported by the lazy chunks: a
 * static import from them would split Menu out of the Control route's chunk (costing its budget).
 */
export type MenuType = typeof MenuComponent;

/** What a LazyStrip hands its chunk. */
export type StripProps = { ctl: ControlCtl; Menu: MenuType };

export type SourcesStripProps = {
  sources: readonly SourceSummary[];
  onstep: (id: string, delta: number) => void;
  autoOf: (s: SourceSummary) => AutoAdvance;
  onauto: (id: string, on: boolean) => void;
  onautosettings: (id: string, auto: AutoAdvance) => void;
  /** An API source's Retry / Pause reply. */
  onapi?: (s: SourceSummary) => void;
};

function cached<P extends Record<string, unknown>>(load: () => Promise<{ default: unknown }>) {
  let mod: Promise<Component<P>> | undefined;
  return (): Promise<Component<P>> => {
    mod ??= load().then((m) => m.default as Component<P>);
    mod.catch(() => (mod = undefined));
    return mod;
  };
}

export const loadAutomation = cached<StripProps>(() => import("$lib/components/recipes/Automation.svelte"));
export const loadSourcesStrip = cached<SourcesStripProps>(() => import("./SourcesStrip.svelte"));

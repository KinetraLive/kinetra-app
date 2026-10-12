// Lazy chunks of Control's Automation strip: each panel kind, the gallery, the remove and fix dialogs load on first use (cached; a failed load is retried next time).

import type { Component } from "svelte";
import type { ControlCtl } from "$lib/components/control/control.svelte";
import type MenuComponent from "$lib/components/control/Menu.svelte";
import type { LiveScreenView, RecipeInstance, RecipePanel, SourceSummary } from "$lib/types";
import type { Broken } from "./automation";

/**
 * Control's Menu, handed down by the static strip rather than imported here: a static import from
 * these lazy chunks would split Menu out of the Control route's chunk (costing its budget).
 */
export type MenuType = typeof MenuComponent;

export type AutomationProps = { ctl: ControlCtl; Menu: MenuType };

export type SchedulePanelProps = {
  Menu: MenuType;
  sourceId: string;
  kind: "schedule" | "countdown";
  name: string;
  /** The source summary's `updatedAt`: a change reloads the run of show (titles, overrides). */
  updatedAt: string | null;
};
export type SlideshowPanelProps = { ctl: ControlCtl; sourceId: string };
export type GalleryProps = {
  open: boolean;
  onclose: () => void;
  screens: readonly LiveScreenView[];
  /** Preselected existing screen. */
  selectedScreenId: string | null;
  oninstalled: (inst: RecipeInstance) => void;
  /** The dialog's title (default "Add recipe"; Control says "Add automation"). */
  title?: string;
};
export type RemoveProps = {
  recipe: RecipeInstance;
  onclose: () => void;
  onremoved: (deleteCreated: boolean) => void;
};

export type FixProps = {
  broken: Broken;
  /** The project's sources (the picker keeps those of the kind it runs on). */
  sources: readonly SourceSummary[];
  /** Name of the screen it was installed on. */
  screenName: string;
  onclose: () => void;
  onfixed: (inst: RecipeInstance) => void;
  /** Opens the remove dialog instead. */
  ondelete: () => void;
};

type Loader<P extends Record<string, unknown>> = () => Promise<Component<P>>;

function cached<P extends Record<string, unknown>>(load: () => Promise<{ default: unknown }>): Loader<P> {
  let mod: Promise<Component<P>> | undefined;
  return () => {
    mod ??= load().then((m) => m.default as Component<P>);
    mod.catch(() => (mod = undefined));
    return mod;
  };
}

export const loadSchedulePanel = cached<SchedulePanelProps>(() => import("./SchedulePanel.svelte"));
export const loadSlideshowPanel = cached<SlideshowPanelProps>(() => import("./SlideshowPanel.svelte"));
export const loadGallery = cached<GalleryProps>(() => import("./RecipeGallery.svelte"));
export const loadRemove = cached<RemoveProps>(() => import("./RemoveRecipe.svelte"));
export const loadFix = cached<FixProps>(() => import("./FixRecipe.svelte"));

export const PANEL_LABEL: Record<RecipePanel, string> = {
  schedule: "Schedule",
  countdown: "Countdown",
  slideshow: "Slideshow",
};

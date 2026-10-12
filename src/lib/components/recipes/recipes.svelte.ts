// The installed recipes of the open project, for Control's Automation strip (lazy chunk).
//
//   const recipes = new RecipesState();
//   $effect(() => recipes.connect());   // loads the list
//
// A recipe's automation is its flows (`flowIds`, and `origin` on each flow); they live in
// ControlCtl's FlowsState, which follows `flow-state`. Which rows are expanded (collapsed by default)
// is remembered for the session (module state), so leaving Control and coming back keeps the layout.

import { untrack } from "svelte";
import { toAppError } from "$lib/api";
import { listRecipeInstances } from "$lib/api/recipes";
import { toasts } from "$lib/stores/toasts.svelte";
import type { RecipeInstance } from "$lib/types";

/** The row key of a screen's own rules (not of any recipe). */
export const OTHER_ROW = "__other";

/** Expanded row ids (recipe ids, or OTHER_ROW; session only). */
let expandedIds = $state.raw<ReadonlySet<string>>(new Set());

/** Collapses every row (tests). */
export function collapseAll(): void {
  expandedIds = new Set();
}

export class RecipesState {
  /** Installed recipes, in install order. */
  list = $state.raw<RecipeInstance[]>([]);
  loaded = $state(false);

  get expanded(): ReadonlySet<string> {
    return expandedIds;
  }

  connect(): void {
    untrack(() => void this.load());
  }

  async load(): Promise<void> {
    try {
      this.list = await listRecipeInstances();
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't load the recipes: ${toAppError(e).message}` });
    } finally {
      this.loaded = true;
    }
  }

  get(id: string): RecipeInstance | undefined {
    return this.list.find((r) => r.id === id);
  }

  /** Replaces an instance (or adds it at the end). */
  put(inst: RecipeInstance): void {
    const known = this.list.some((r) => r.id === inst.id);
    this.list = known ? this.list.map((r) => (r.id === inst.id ? inst : r)) : [...this.list, inst];
  }

  drop(id: string): void {
    this.list = this.list.filter((r) => r.id !== id);
  }

  toggle = (id: string): void => {
    const next = new Set(expandedIds);
    if (!next.delete(id)) next.add(id);
    expandedIds = next;
  };

  expand(id: string): void {
    if (!expandedIds.has(id)) this.toggle(id);
  }
}

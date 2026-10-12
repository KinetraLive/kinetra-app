// Recipes (pre-made automation) and schedule operator actions.
// Split out of the core `$lib/api` so only the routes that use these commands load them.

import type {
  InstallRecipe,
  RecipeInfo,
  RecipeInstance,
  ScheduleAction,
  ScheduleView,
} from "../types";
import { call } from "./index";

export const listRecipes = () => call<RecipeInfo[]>("list_recipes");
export const listRecipeInstances = () => call<RecipeInstance[]>("list_recipe_instances");
export const installRecipe = (request: InstallRecipe) =>
  call<RecipeInstance>("install_recipe", { request });
export const removeRecipe = (id: string, deleteCreated: boolean) =>
  call<void>("remove_recipe", { id, deleteCreated });
/** Points a recipe at another source of the same kind (its flows follow). */
export const relinkRecipeSource = (id: string, sourceId: string) =>
  call<RecipeInstance>("relink_recipe_source", { id, sourceId });
export const scheduleView = (sourceId: string) => call<ScheduleView>("schedule_view", { sourceId });
export const scheduleAction = (sourceId: string, action: ScheduleAction) =>
  call<ScheduleView>("schedule_action", { sourceId, action });

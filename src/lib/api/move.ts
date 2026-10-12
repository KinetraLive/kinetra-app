// Moving the open project folder (Settings > Location).
// Split out of the core `$lib/api` so only the routes that use these commands load them.

import type { Project } from "../types";
import { call } from "./index";

export const moveProject = (dest: string, deleteOriginal: boolean) =>
  call<Project>("move_project", { dest, deleteOriginal });
export const cancelMove = () => call<void>("cancel_move");

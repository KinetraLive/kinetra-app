// Sources: CRUD and stepping (Next/Prev). A change re-applies every screen bound to the source.
// Split out of the core `$lib/api` so only the routes that use these commands load them.

import type { ScheduleDoc, SourceDoc, SourceStep, SourceSummary } from "../types";
import { call } from "./index";

/** Sorted by name, with `usedBy` (screen ids). */
export const listSources = () => call<SourceSummary[]>("list_sources");
/** A text source (check `SourceSummary.kind` first); schedules: `getSchedule`. */
export const getSource = (id: string) => call<SourceDoc>("get_source", { id });
/** A schedule source (same command, schedule shape). */
export const getSchedule = (id: string) => call<ScheduleDoc>("get_source", { id });
/** A new text source with one column and no rows. */
export const createSource = (name: string) => call<SourceDoc>("create_source", { name });
export const saveSource = (doc: SourceDoc) => call<SourceDoc>("save_source", { doc });
export const saveSchedule = (doc: ScheduleDoc) => call<ScheduleDoc>("save_source", { doc });
/** A new schedule source with no items. */
export const createSchedule = (name: string) => call<ScheduleDoc>("create_schedule", { name });
export const renameSource = (id: string, name: string) =>
  call<SourceSummary>("rename_source", { id, name });
export const duplicateSource = (id: string, name: string) =>
  call<SourceDoc>("duplicate_source", { id, name });
export const deleteSource = (id: string) => call<void>("delete_source", { id });
/** Next/Prev (`{ to: "by", delta }`) or go to a row (`{ to: "index", index }`). */
export const stepSource = (id: string, step: SourceStep) =>
  call<SourceSummary>("step_source", { id, step });

// API sources: ./api-sources.ts (kept out of this module, which Control and the flow editor load).

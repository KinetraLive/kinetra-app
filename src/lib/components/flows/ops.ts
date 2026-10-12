// Flow operations used only by the lazy chunks (Automation strip, linear editor), kept
// out of FlowsState so the Control route's eager chunk carries just the list, the run state and
// Run / Stop. Each updates the shared FlowsState with the command's reply.

import { toAppError } from "$lib/api";
import { createFlow, deleteFlow, duplicateFlow, saveFlow, setFlowArmed } from "$lib/api/flows";
import type { ControlCtl } from "$lib/components/control/control.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import type { FlowInfo, NewFlow, SaveFlow } from "$lib/types";
import type { FlowsState } from "./flows.svelte";

/** Arm or disarm (optimistic; rolled back on failure, only when nothing newer arrived). */
export async function setArmed(flows: FlowsState, id: string, armed: boolean): Promise<void> {
  const before = flows.get(id);
  if (!before) return;
  const optimistic = { ...before, armed, running: armed && before.running };
  flows.put(optimistic);
  try {
    flows.put(await setFlowArmed(id, armed));
  } catch (e) {
    if (flows.get(id) === optimistic) flows.put(before);
    toasts.push({ kind: "error", message: `Couldn't ${armed ? "arm" : "disarm"} ${before.name}: ${toAppError(e).message}` });
  }
}

/** Creates (validated and deployed); throws for the editor to show. */
export async function create(flows: FlowsState, flow: NewFlow): Promise<FlowInfo> {
  const f = await createFlow(flow);
  flows.put(f);
  return f;
}

/** Saves and deploys; throws for the editor to show. */
export async function save(flows: FlowsState, flow: SaveFlow): Promise<FlowInfo> {
  const f = await saveFlow(flow);
  flows.put(f);
  return f;
}

export async function duplicate(flows: FlowsState, id: string): Promise<void> {
  try {
    flows.put(await duplicateFlow(id));
  } catch (e) {
    toasts.push({ kind: "error", message: `Couldn't duplicate it: ${toAppError(e).message}` });
  }
}

/** Deletes a flow and drops its hotkey (a failure toasts and keeps both). */
export async function remove(ctl: ControlCtl, id: string): Promise<void> {
  try {
    await deleteFlow(id);
  } catch (e) {
    toasts.push({ kind: "error", message: `Couldn't delete it: ${toAppError(e).message}` });
    return;
  }
  ctl.flows.drop(id);
  ctl.removeHotkey(`flow/${id}`);
}

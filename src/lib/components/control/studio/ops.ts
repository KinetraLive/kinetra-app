// Studio takes and cues (lazy studio chunk). Every one goes through `studioOp`, the core's single
// studio entry point; its reply (the new PreviewView) goes into `previews` at once. A cue toggle is
// optimistic and rolled back on failure (only when nothing newer arrived meanwhile). Also Update on
// air / Discard for one layer, Play in / Play out (Preview only), the take summary and turning
// Studio mode off (with the exit question).

import { mount, unmount } from "svelte";
import { toAppError } from "$lib/api";
import { studioOp } from "$lib/api/live";
import { live } from "$lib/stores/live.svelte";
import { previews } from "$lib/stores/preview.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import type { PreviewView, RehearseKind, StudioOp, TakeTransition } from "$lib/types";
import type { StudioCtl, StudioExit } from "./studio.svelte";

/** Whether a layer shows in the screen's Preview (its Program state when the Preview is unknown). */
export function inPreview(pv: PreviewView | undefined, layerId: string, onAir: boolean): boolean {
  return pv?.layers.find((l) => l.id === layerId)?.onAir ?? onAir;
}

async function op(screenId: string, o: StudioOp, what: string): Promise<boolean> {
  try {
    previews.put(await studioOp(screenId, o));
    return true;
  } catch (e) {
    toasts.push({ kind: "error", message: `Couldn't ${what}: ${toAppError(e).message}` });
    return false;
  }
}

/** The screen's Preview to Program (`transition` null = the project default). */
export const take = (screenId: string, transition: TakeTransition | null = null): Promise<boolean> =>
  op(screenId, { op: "take", transition }, "take the preview");

/** Drops every cue of the screen: Preview mirrors Program again. */
export const clearPreview = (screenId: string): Promise<boolean> => op(screenId, { op: "clear" }, "clear the preview");

/** Cues a layer in or out of Preview (back to "as Program" when that is what Program shows). */
export async function cue(screenId: string, layerId: string): Promise<void> {
  const program = live.screens.get(screenId)?.layers.find((l) => l.id === layerId);
  if (!program) return;
  const pv = previews.views.get(screenId);
  const visible = !inPreview(pv, layerId, program.onAir);
  const want = visible === program.onAir ? null : visible;
  let optimistic: PreviewView | null = null;
  if (pv) {
    const rest = pv.cues.filter((c) => c.layerId !== layerId);
    const old = pv.cues.find((c) => c.layerId === layerId);
    const keep = want !== null || old?.staged === true;
    optimistic = {
      ...pv,
      cues: keep ? [...rest, { layerId, staged: false, programChanged: false, ...old, visible: want }] : rest,
      layers: pv.layers.map((l) => (l.id === layerId ? { ...l, onAir: visible } : l)),
    };
    previews.put(optimistic, true);
  }
  try {
    previews.put(await studioOp(screenId, { op: "cue", layerId, visible: want }));
  } catch (e) {
    if (pv && previews.views.get(screenId) === optimistic) previews.put(pv, true);
    toasts.push({ kind: "error", message: `Couldn't cue ${program.name}: ${toAppError(e).message}` });
  }
}

function pendingNames(): string[] {
  const out: string[] = [];
  for (const [id, v] of previews.views) if (v.cues.length > 0) out.push(live.screens.get(id)?.screen.name ?? id);
  return out;
}

function ask(s: StudioCtl): void {
  s.asking = pendingNames();
}

/**
 * Turns Studio mode off. Without `exit`: asks Take / Keep / Discard when a Preview differs from
 * Program (also when the server refuses because of a Preview we didn't know about).
 */
export async function turnOff(s: StudioCtl, exit: StudioExit | null = null): Promise<void> {
  if (!exit && pendingNames().length > 0) return ask(s);
  const e = await s.save({ ...s.settings, enabled: false }, exit);
  if (!e) {
    previews.clear();
    return;
  }
  if (!exit) {
    await previews.load([...live.screens.keys()], true);
    if (pendingNames().length > 0) return ask(s);
  }
  toasts.push({ kind: "error", message: `Couldn't turn Studio mode off: ${toAppError(e).message}` });
}

/** "Update on air" asks once per app session (the chunk lives as long as the window). */
let updateConfirmed = false;

/** Asks "Update the on-air graphic?" (a ConfirmDialog on <body>); true to go ahead. */
function confirmUpdate(name: string): Promise<boolean> {
  if (updateConfirmed) return Promise.resolve(true);
  return import("$lib/components/ConfirmDialog.svelte").then(
    ({ default: ConfirmDialog }) =>
      new Promise<boolean>((resolve) => {
        const done = (ok: boolean) => {
          if (ok) updateConfirmed = true;
          resolve(ok);
          queueMicrotask(() => void unmount(instance));
        };
        const instance = mount(ConfirmDialog, {
          target: document.body,
          props: {
            title: "Update the on-air graphic?",
            message: `Your changes to ${name} go on air now, without a take. You won't be asked again until Kinetra restarts.`,
            confirmLabel: "Update on air",
            onconfirm: () => done(true),
            oncancel: () => done(false),
          },
        });
      }),
  );
}

/** Update on air: one on-air layer's Preview changes to Program now, settled (asks once per session). */
export async function updateOnAir(screenId: string, layerId: string, name: string): Promise<boolean> {
  if (!(await confirmUpdate(name))) return false;
  return op(screenId, { op: "update", layerId }, `update ${name} on air`);
}

/** Drops one layer's Preview changes (its cue stays). */
export const discard = (screenId: string, layerId: string, name: string): Promise<boolean> =>
  op(screenId, { op: "discard", layerId }, `discard the changes to ${name}`);

/** Play in / Play out in Preview only. */
export const rehearse = (screenId: string, layerIds: string[], kind: RehearseKind): Promise<boolean> =>
  op(screenId, { op: "rehearse", layerIds, kind }, kind === "in" ? "play it in" : "play it out");

/** What a take of the screen's Preview would do: layers coming in, going out, and on-air layers whose content changes. */
export function takeSummary(pv: PreviewView | undefined, program: readonly { id: string; onAir: boolean }[]): { in: number; out: number; updated: number } {
  const n = { in: 0, out: 0, updated: 0 };
  for (const c of pv?.cues ?? []) {
    const onAir = program.find((l) => l.id === c.layerId)?.onAir ?? false;
    if (c.visible === true && !onAir) n.in++;
    else if (c.visible === false && onAir) n.out++;
    else if (c.staged && onAir) n.updated++;
  }
  return n;
}

/** "2 in · 1 out · 1 updated", "" when the take would change nothing on air. */
export function summaryText(n: { in: number; out: number; updated: number }): string {
  return [n.in && `${n.in} in`, n.out && `${n.out} out`, n.updated && `${n.updated} updated`].filter(Boolean).join(" · ");
}

// "Update to the latest look" (a lazy chunk: UpdateNotice and the palette command load it).
//
//   updateLook({ id, name }, { before?, after? })
//
// Asks first (what is replaced, what is kept, that a backup is taken), saves the open editors
// (`before`, plus the style hooks: the layout editor flushes and reloads), then the backend takes
// a `pre-update` backup and copies the preset's latest looks in (screens using the template follow
// the live rules). A toast with Undo follows for 10 s; Undo puts the template back from that
// backup. `after` gets the template as saved, after the update and after an undo; the window
// event TEMPLATE_UPDATED tells everyone else (the templates list reloads).

import { mount, unmount } from "svelte";
import { toAppError } from "$lib/api";
import { undoTemplateUpdate, updateTemplateFromPreset } from "$lib/api/template-update";
import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
import { afterStyle, beforeStyle } from "$lib/components/styles/styles";
import { toasts } from "$lib/stores/toasts.svelte";
import { TEMPLATE_UPDATED } from "$lib/templates-common";
import type { TemplateDoc, TemplateUpdated } from "$lib/types";

/** How long the result toast (and its Undo) stays up. */
export const UNDO_MS = 10_000;

export interface UpdateTarget {
  id: string;
  name: string;
}

export interface UpdateHooks {
  /** Runs before the backend writes (an editor saves its working copy). */
  before?: () => Promise<unknown>;
  /** The template as saved, after the update and after an undo (reload, refresh lists). */
  after?: (doc: TemplateDoc) => void;
}

export const confirmTitle = (name: string) => `Update “${name}” to the latest look?`;
export const CONFIRM_MESSAGE =
  "Your edits to the built-in looks' HTML and CSS are replaced. Variants you added and field values are kept. A backup is taken first.";

/** The confirm dialog, mounted on <body> while open; resolves true for Update. */
function confirm(name: string): Promise<boolean> {
  const back = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  return new Promise((resolve) => {
    const done = (ok: boolean) => {
      resolve(ok);
      queueMicrotask(() => {
        void unmount(dialog);
        if (back?.isConnected) back.focus();
      });
    };
    const dialog = mount(ConfirmDialog, {
      target: document.body,
      props: {
        title: confirmTitle(name),
        message: CONFIRM_MESSAGE,
        confirmLabel: "Update",
        onconfirm: () => done(true),
        oncancel: () => done(false),
      },
    });
  });
}

function announce(doc: TemplateDoc, hooks: UpdateHooks) {
  hooks.after?.(doc);
  dispatchEvent(new CustomEvent(TEMPLATE_UPDATED, { detail: doc }));
}

async function undo(t: UpdateTarget, backup: string, hooks: UpdateHooks): Promise<void> {
  try {
    await Promise.all([hooks.before?.(), beforeStyle()]);
    const doc = await undoTemplateUpdate(t.id, backup);
    announce(doc, hooks);
    toasts.push({ kind: "info", message: `“${t.name}” has its previous look again` });
  } catch (e) {
    toasts.push({
      kind: "error",
      message: `Couldn't undo the update of “${t.name}”: ${toAppError(e).message}. Backup ${backup} has a copy.`,
    });
  } finally {
    afterStyle();
  }
}

let busy = false;

/** Asks, updates, toasts with Undo. Resolves with the template as saved, or null (canceled or failed). */
export async function updateLook(t: UpdateTarget, hooks: UpdateHooks = {}): Promise<TemplateDoc | null> {
  if (busy) return null;
  busy = true;
  try {
    if (!(await confirm(t.name))) return null;
    let r: TemplateUpdated;
    try {
      await Promise.all([hooks.before?.(), beforeStyle()]);
      r = await updateTemplateFromPreset(t.id);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't update “${t.name}”: ${toAppError(e).message}` });
      return null;
    } finally {
      afterStyle();
    }
    announce(r.doc, hooks);
    const backup = r.backup.id;
    toasts.push({
      kind: "success",
      message: `“${t.name}” has the latest look`,
      timeoutMs: UNDO_MS,
      action: { label: "Undo", run: () => void undo(t, backup, hooks) },
    });
    return r.doc;
  } finally {
    busy = false;
  }
}

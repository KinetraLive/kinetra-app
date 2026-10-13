// The native menu sync (a lazy chunk; the root layout loads it after first paint, in the main
// window only). Registers the app commands (./commands.ts) and the "Open recent" and Output
// providers for the session, keeps the menu equal to `buildMenuSpec` of the current registry, bindings, context,
// recents, mode and cues, and runs menu clicks.
//
// One effect rebuilds the spec when anything it read changed (Svelte batches changes into one run
// per microtask); the JSON is compared with the last one sent and `set_app_menu` is called only on a
// change, so a route change or keystroke that doesn't change the menu sends nothing.
//
// The window's close button (`close-requested`, sent only with a project open; the picker closes
// at once) is answered at once, then runs File › Exit (`app.exit`: asks first while anything is on
// air or outputs are showing). Without the answer the backend exits on a second close within 5 s.
//
// A click (`menu` { id }) runs the command through the registry's rules (`when`, `disabled`), like
// the palette but without recording Recent. Clear all and Panic run at once, like their keys: an
// operator picking them from the Show menu means it (the palette's second Enter guards against a
// mistyped search, which a menu click isn't).

import { untrack } from "svelte";
import { on, toAppError } from "$lib/api";
import { closeRequestAck, setAppMenu } from "$lib/api/app";
import { menuLabel } from "$lib/components/outputs/labels";
import { uiMode } from "$lib/stores/mode.svelte";
import { prefs } from "$lib/stores/prefs.svelte";
import { project, uiState } from "$lib/stores/project.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { isMac } from "$lib/platform";
import { keysFor } from "../bindings.svelte";
import { commands, currentContext, getCommand, isAllowed, registerCommands, registerProvider, type Command } from "../registry.svelte";
import { appCommands, OUTPUT_PREFIX, outputsProvider, recentsProvider } from "./commands";
import { buildMenuSpec, type MenuInput } from "./spec";

/** What `buildMenuSpec` reads, from the live registry and stores (reactive in an effect). */
export function menuInput(mac = isMac()): MenuInput {
  const ctx = currentContext();
  const list = commands();
  const recents = recentsProvider.items(ctx);
  const byId = new Map<string, Command>();
  for (const c of list) byId.set(c.id, c);
  for (const c of recents) byId.set(c.id, c);
  const configs = prefs.value?.outputs ?? [];
  const outputs = outputsProvider.items(ctx).map((c) => {
    const o = configs.find((x) => OUTPUT_PREFIX + x.id === c.id);
    byId.set(c.id, c);
    return { id: c.id, text: o ? menuLabel(o) : c.id, checked: !!o?.enabled };
  });
  return {
    ctx,
    mac,
    command: (id) => byId.get(id),
    keys: keysFor,
    cues: list.filter((c) => c.id.startsWith("cue.go:")),
    recents,
    outputs,
    uiMode: uiMode(),
    sidebarPinned: uiState.value?.sidebarPinned ?? false,
  };
}

/** Runs a menu item's command (see the header). True when it ran. */
export async function runMenuCommand(id: string): Promise<boolean> {
  const c = getCommand(id);
  const ctx = currentContext();
  if (!c?.run || !isAllowed(c, ctx)) return false;
  try {
    await c.run(ctx);
  } catch (e) {
    toasts.push({ kind: "error", message: toAppError(e).message });
  }
  return true;
}

let closing = false;

/** `close-requested`: answers the backend, then runs File › Exit (once at a time: a close while
 *  the question is up only answers). */
export async function onCloseRequested(): Promise<void> {
  await closeRequestAck().catch(() => {});
  if (closing) return;
  closing = true;
  try {
    await runMenuCommand("app.exit");
  } finally {
    closing = false;
  }
}

/** Starts the sync; returns the function that stops it (and unregisters the commands). */
export function startMenuSync(send: (spec: ReturnType<typeof buildMenuSpec>) => Promise<unknown> = setAppMenu): () => void {
  const offCommands = registerCommands(appCommands());
  const offRecents = registerProvider(recentsProvider);
  const offOutputs = registerProvider(outputsProvider);
  const offMenu = on("menu", ({ id }) => void runMenuCommand(id));
  const offClose = on("close-requested", () => void onCloseRequested());
  const offChecked = project.watchRecents();
  let last = "";
  const stop = $effect.root(() => {
    // Opening or closing a project reorders the recents: reload them.
    $effect(() => {
      void project.current?.path;
      untrack(() => project.loadRecents().catch(() => {}));
    });
    $effect(() => {
      const spec = buildMenuSpec(menuInput());
      const json = JSON.stringify(spec);
      if (json === last) return;
      last = json;
      untrack(() =>
        send(spec).catch(() => {
          // Not sent (no backend, or it failed): try again on the next change.
          if (last === json) last = "";
        }),
      );
    });
  });
  return () => {
    stop();
    offMenu();
    offClose();
    offChecked();
    offRecents();
    offOutputs();
    offCommands();
  };
}

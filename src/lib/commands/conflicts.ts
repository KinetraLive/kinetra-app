// Key conflicts and the project table's bindings, for the places that show them (Preferences >
// Keyboard shortcuts, Control's layer hotkey). Its own module so the shell, which only resolves
// keys (./bindings.svelte.ts), doesn't carry it: it loads with those lazy chunks.

import { uiState } from "$lib/stores/project.svelte";
import { bindingIn, bindingTable, commandOfProjectKey, FLOW_KEY_PREFIX, type Binding, type Conflict } from "./bindings.svelte";
import type { KeyScope } from "./registry.svelte";

/** Every layer/cue binding in the project table (also of commands not registered right now). */
export function projectBindings(): Binding[] {
  const out = new Map<string, Binding>();
  for (const [key, combo] of Object.entries(uiState.value?.control.hotkeys ?? {})) {
    const id = commandOfProjectKey(key);
    // "flow/<id>" wins over a leftover "macro/<id>".
    if (id && (!out.has(id) || key.startsWith(FLOW_KEY_PREFIX))) out.set(id, { commandId: id, keys: [combo], source: "project" });
  }
  return [...out.values()];
}

interface Entry {
  id: string;
  scope: KeyScope;
  keys: string;
}

const overlap = (a: KeyScope, b: KeyScope) => a === "global" || b === "global" || a === b;

/**
 * Every conflict among registered commands' effective keys and the project keys of layer/cue
 * commands not registered right now (e.g. while Control isn't open; key scope "control"). Cached
 * with the binding table.
 */
export function conflicts(): readonly Conflict[] {
  const c = bindingTable();
  if (c.conflicts) return c.conflicts;
  const byKeys = new Map<string, Entry[]>();
  const add = (e: Entry) => {
    const at = byKeys.get(e.keys);
    if (at) at.push(e);
    else byKeys.set(e.keys, [e]);
  };
  const registered = new Set<string>();
  for (const cmd of c.list) {
    registered.add(cmd.id);
    for (const keys of bindingIn(cmd.id, cmd.defaultKeys, c.p, c.u).keys) add({ id: cmd.id, scope: cmd.keyScope ?? "global", keys });
  }
  for (const b of projectBindings()) if (!registered.has(b.commandId)) add({ id: b.commandId, scope: "control", keys: b.keys[0] });
  const out: Conflict[] = [];
  for (const [keys, entries] of byKeys) {
    if (entries.length < 2) continue;
    const ids = new Set<string>();
    for (const a of entries) for (const b of entries) if (a !== b && a.id !== b.id && overlap(a.scope, b.scope)) ids.add(a.id);
    if (ids.size > 1) out.push({ keys, commandIds: [...ids] });
  }
  c.conflicts = out;
  return out;
}

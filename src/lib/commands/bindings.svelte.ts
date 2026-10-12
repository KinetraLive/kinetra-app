// Key bindings: which keys run which command. Separate from the commands: a user-editable table
// (command id → keys) overrides a command's `defaultKeys`.
//   - Project table, for layer and cue commands: `ui-state.json` control.hotkeys, format unchanged
//     ("<screenId>/<layerId>" ⇄ "live.toggle:<screenId>/<layerId>", "flow/<flowId>" ⇄
//     "cue.go:<flowId>"; a leftover "macro/<id>" counts as "flow/<id>"). One F1–F12 key (with or
//     without Shift) per target and one target per key.
//   - Machine table, for everything else: `prefs.json` keybindings ([] = unbound).
// Conflicts (same keys, overlapping key scopes; "global" overlaps every scope) are detected and
// reported, never resolved by registration order (`conflicts()` and `projectBindings()` are in
// ./conflicts.ts, so only the lazy chunks that show them load them). Part of the shell: lookups are
// cached per change of the registry or either table, so a keydown is a map lookup.

import { t } from "$lib/i18n";
import { prefs } from "$lib/stores/prefs.svelte";
// Through the project store: both stores stay in the one chunk the picker loads.
import { uiState } from "$lib/stores/project.svelte";
import { KEYS_PER_COMMAND, type Prefs, type UiState } from "$lib/types";
import { normalizeKeys } from "./keys";
import { registry, type Command } from "./registry.svelte";

/** One command's keys after overrides. `source`: where they come from. */
export interface Binding {
  commandId: string;
  keys: readonly string[];
  source: "default" | "machine" | "project";
}

export interface Conflict {
  keys: string;
  /** Two or more command ids whose key scopes overlap. */
  commandIds: readonly string[];
}

// ---- The project table (control.hotkeys) -------------------------------------------------------

export const LAYER_COMMAND = "live.toggle:";
export const CUE_COMMAND = "cue.go:";
export const FLOW_KEY_PREFIX = "flow/";
/** The legacy prefix of the same keys (macros became flows with the same id). */
export const LEGACY_KEY_PREFIX = "macro/";
/** Project keys: F1–F12, optionally with Shift. */
export const PROJECT_KEY_RE = /^(Shift\+)?F([1-9]|1[0-2])$/;

type Hotkeys = Readonly<Record<string, string>>;

/** The flow id of a flow key (or a leftover "macro/<id>" key), else null. */
export function flowOfKey(key: string): string | null {
  for (const p of [FLOW_KEY_PREFIX, LEGACY_KEY_PREFIX]) {
    if (key.startsWith(p)) return key.length > p.length ? key.slice(p.length) : null;
  }
  return null;
}

/** A layer key split into its ids; null for a flow key or a malformed key. */
export function splitHotkeyKey(key: string): { screenId: string; layerId: string } | null {
  if (flowOfKey(key) !== null) return null;
  const i = key.indexOf("/");
  if (i <= 0 || i === key.length - 1) return null;
  return { screenId: key.slice(0, i), layerId: key.slice(i + 1) };
}

/** The control.hotkeys key of a layer or cue command; null for any other command. */
export function projectKeyOf(commandId: string): string | null {
  if (commandId.startsWith(LAYER_COMMAND)) {
    const k = commandId.slice(LAYER_COMMAND.length);
    return splitHotkeyKey(k) ? k : null;
  }
  if (commandId.startsWith(CUE_COMMAND)) {
    const f = commandId.slice(CUE_COMMAND.length);
    return f ? FLOW_KEY_PREFIX + f : null;
  }
  return null;
}

/** The command of a control.hotkeys key ("s/l" → "live.toggle:s/l", "flow/f" → "cue.go:f"). */
export function commandOfProjectKey(key: string): string | null {
  const flow = flowOfKey(key);
  if (flow !== null) return CUE_COMMAND + flow;
  return splitHotkeyKey(key) ? LAYER_COMMAND + key : null;
}

/** The key mapped to a project key ("flow/<id>" also finds a leftover "macro/<id>"). */
function projectCombo(hotkeys: Hotkeys, key: string): string | null {
  const flow = key.startsWith(FLOW_KEY_PREFIX) ? key.slice(FLOW_KEY_PREFIX.length) : null;
  return hotkeys[key] ?? (flow ? hotkeys[LEGACY_KEY_PREFIX + flow] : undefined) ?? null;
}

/**
 * Maps `combo` to `key`; a previous owner of the combo loses it (one target per combo). A flow key
 * also replaces that flow's leftover "macro/<id>" key.
 */
export function assignHotkey(hotkeys: Hotkeys, key: string, combo: string): Record<string, string> {
  const flow = flowOfKey(key);
  const legacy = flow === null ? null : LEGACY_KEY_PREFIX + flow;
  const next: Record<string, string> = {};
  for (const [k, c] of Object.entries(hotkeys)) if (c !== combo && k !== key && k !== legacy) next[k] = c;
  next[key] = combo;
  return next;
}

/** Drops a key (a flow key also drops the flow's leftover "macro/<id>" key). */
export function removeHotkey(hotkeys: Hotkeys, key: string): Record<string, string> {
  const next = { ...hotkeys };
  delete next[key];
  const flow = flowOfKey(key);
  if (flow !== null) {
    delete next[FLOW_KEY_PREFIX + flow];
    delete next[LEGACY_KEY_PREFIX + flow];
  }
  return next;
}

// ---- Effective keys ------------------------------------------------------------------------------

export function bindingIn(commandId: string, defaults: readonly string[] | undefined, p: Prefs | null, u: UiState | null): Binding {
  const pk = projectKeyOf(commandId);
  if (pk !== null) {
    const combo = projectCombo(u?.control.hotkeys ?? {}, pk);
    if (combo) return { commandId, keys: [combo], source: "project" };
  } else {
    const m = p?.keybindings?.[commandId];
    if (m) return { commandId, keys: m, source: "machine" };
  }
  return { commandId, keys: defaults ?? [], source: "default" };
}

/** A command's binding: the project table for layer/cue commands, else the machine table, else
 * its `defaultKeys` (of the registered command with this id). */
export function bindingOf(commandId: string): Binding {
  const c = registry.list.find((x) => x.id === commandId);
  return bindingIn(commandId, c?.defaultKeys, prefs.value, uiState.value);
}

/** The effective keys of a command. */
export const keysFor = (commandId: string): readonly string[] => bindingOf(commandId).keys;

// ---- Lookups, cached per change --------------------------------------------------------------------

let cache: {
  list: readonly Command[];
  p: Prefs | null;
  u: UiState | null;
  index: Map<string, Command[]>;
  /** Filled by ./conflicts.ts on first ask. */
  conflicts: readonly Conflict[] | null;
} | null = null;

/** The binding table for the current registry and tables (rebuilt only when one changed). */
export function bindingTable() {
  const list = registry.list;
  const p = prefs.value;
  const u = uiState.value;
  if (cache && cache.list === list && cache.p === p && cache.u === u) return cache;
  const index = new Map<string, Command[]>();
  for (const c of list) {
    for (const k of bindingIn(c.id, c.defaultKeys, p, u).keys) {
      const at = index.get(k);
      if (at) at.push(c);
      else index.set(k, [c]);
    }
  }
  cache = { list, p, u, index, conflicts: null };
  return cache;
}

/** The registered commands bound to `keys` (canonical spelling), in registration order. */
export const commandsForKeys = (keys: string): readonly Command[] => bindingTable().index.get(keys) ?? [];

// ---- Writing ---------------------------------------------------------------------------------------

/**
 * Sets a command's keys in its table (project for layer/cue commands, machine otherwise); `null`
 * restores the default. Never rejects a conflict: it is stored and reported by `conflicts()`.
 * Rejects keys that aren't keys, and project keys other than one F1–F12 (with or without Shift).
 */
export async function setKeys(commandId: string, keys: readonly string[] | null): Promise<void> {
  let norm: string[] | null = null;
  if (keys !== null) {
    norm = [];
    for (const k of keys) {
      const n = normalizeKeys(k);
      if (!n) throw new Error(t("{keys} can't be used as a shortcut.", { keys: k }));
      if (!norm.includes(n)) norm.push(n);
    }
  }
  const pk = projectKeyOf(commandId);
  if (pk !== null) {
    const v = uiState.value;
    if (!v) throw new Error(t("Open a project first."));
    const combo = norm?.[0];
    if (norm && (norm.length > 1 || (combo && !PROJECT_KEY_RE.test(combo)))) {
      throw new Error(t("Layer and cue keys are F1 to F12, with or without Shift."));
    }
    const hotkeys = combo ? assignHotkey(v.control.hotkeys, pk, combo) : removeHotkey(v.control.hotkeys, pk);
    await uiState.update({ ...v, control: { ...v.control, hotkeys } });
    return;
  }
  const p = prefs.value;
  if (!p) throw new Error(t("Preferences aren't loaded yet."));
  const table = { ...p.keybindings };
  // Keys equal to a registered command's defaults are stored as "default".
  const reg = registry.list.find((c) => c.id === commandId);
  const defaults = reg?.defaultKeys ?? [];
  const same = !!reg && norm !== null && norm.length === defaults.length && norm.every((k, i) => k === defaults[i]);
  if (norm === null || same) delete table[commandId];
  else table[commandId] = norm.slice(0, KEYS_PER_COMMAND);
  await prefs.update({ ...p, keybindings: table });
}

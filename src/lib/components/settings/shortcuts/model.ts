// Preferences > Keyboard shortcuts, the pure part: rows grouped by palette section, the search
// filter, conflicts per row, and what recording a key writes. No Svelte, no I/O.

import type { Binding, Conflict } from "$lib/commands/bindings.svelte";
import { eventKeys, type KeyLike } from "$lib/commands/keys";
import type { KeyScope, Section } from "$lib/commands/registry.svelte";
import { KEYS_PER_COMMAND } from "$lib/types";

export type RowSection = Exclude<Section, "recent">;

export interface ShortcutRow {
  /** The command id (bindings are stored by id). */
  id: string;
  /** Translated title. */
  title: string;
  section: RowSection;
  /** Where its keys work. */
  scope: KeyScope;
  binding: Binding;
  /** A layer or cue key: one F1–F12 (Shift optional), saved with the project. */
  project: boolean;
}

export const SECTION_ORDER: readonly RowSection[] = ["live", "goto", "create", "edit", "items", "settings"];

export const SECTION_LABEL: Record<RowSection, string> = {
  live: "Live",
  goto: "Go to",
  create: "Create",
  edit: "Edit",
  items: "Project items",
  settings: "Settings",
};

/** Where a key scope's keys work, as the row says it (global: nothing). */
export const SCOPE_LABEL: Partial<Record<KeyScope, string>> = {
  control: "In Control",
  screenEditor: "In the layout editor",
  flowEditor: "In the flow editor",
  templateEditor: "In the template editor",
  screens: "In Screens",
  templates: "In Templates",
  sources: "In Sources",
  flows: "In Flows",
  dashboard: "On the dashboard",
  settings: "In Preferences",
};

export interface RowGroup {
  section: RowSection;
  rows: ShortcutRow[];
}

const fold = (s: string) => s.toLocaleLowerCase();

/**
 * Rows in section order (rows keep their given order inside a section), filtered by `query`:
 * every word must appear in the title, a key or where it works (case-insensitive). Empty groups
 * are left out.
 */
export function groupRows(rows: readonly ShortcutRow[], query: string, scopeText: (s: KeyScope) => string = (s) => SCOPE_LABEL[s] ?? ""): RowGroup[] {
  const words = fold(query).split(/\s+/).filter(Boolean);
  const by = new Map<RowSection, ShortcutRow[]>();
  for (const r of rows) {
    if (words.length > 0) {
      const hay = fold(`${r.title} ${r.binding.keys.join(" ")} ${scopeText(r.scope)}`);
      if (!words.every((w) => hay.includes(w))) continue;
    }
    const at = by.get(r.section);
    if (at) at.push(r);
    else by.set(r.section, [r]);
  }
  return SECTION_ORDER.filter((s) => by.has(s)).map((section) => ({ section, rows: by.get(section) ?? [] }));
}

/** Command id → the conflicts it is part of. */
export function conflictsById(list: readonly Conflict[]): Map<string, Conflict[]> {
  const out = new Map<string, Conflict[]>();
  for (const c of list) {
    for (const id of c.commandIds) {
      const at = out.get(id);
      if (at) at.push(c);
      else out.set(id, [c]);
    }
  }
  return out;
}

/** What a keydown means while recording: cancel (Esc alone), a binding, or nothing yet (a lone
 * modifier). */
export function recordedKey(e: KeyLike): { kind: "cancel" } | { kind: "keys"; keys: string } | null {
  if (e.key === "Escape" && !e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey) return { kind: "cancel" };
  const keys = eventKeys(e)[0];
  return keys ? { kind: "keys", keys } : null;
}

/**
 * The keys to save after recording `keys` for a row: a layer / cue row has one key (replaced);
 * other commands gain it (most recent last; the oldest goes past KEYS_PER_COMMAND), or it replaces
 * `replacing` (fixing a conflict changes that key).
 */
export function keysAfterRecording(row: ShortcutRow, keys: string, replacing?: string): string[] {
  if (row.project) return [keys];
  const rest = row.binding.keys.filter((k) => k !== keys && k !== replacing);
  return [...rest, keys].slice(-KEYS_PER_COMMAND);
}

/** The keys to save after removing `keys` from a row; null removes a layer / cue key. An empty
 * list leaves the command without keys (its defaults no longer apply). */
export function keysAfterRemoving(row: ShortcutRow, keys: string): string[] | null {
  if (row.project) return null;
  return row.binding.keys.filter((k) => k !== keys);
}

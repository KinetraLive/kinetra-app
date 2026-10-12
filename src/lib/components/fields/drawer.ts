// The list drawer's pure parts: what it edits, the "Changes affect N layers." note, and following
// a data table's current row through the sheet's moves and deletes. No Svelte, no DOM.

import { LIST_ROWS_MAX, LIST_VALUE_MAX, SOURCE_CELL_MAX, SOURCE_ROWS_MAX, type ScreenDoc, type SourceColumn, type SourceKindName } from "$lib/types";
import type { SheetLimits } from "$lib/components/sheet/types";

/** What the drawer edits: a layer's own list value, or the data table a list is linked to. */
export type ListDrawerTarget =
  | {
      kind: "list";
      /** The field's label (the drawer's title). */
      label: string;
      columns: readonly SourceColumn[];
      /** The list value (JSON) when the drawer opens. */
      value: string;
      /** Every change as the new list value (the host applies it: Control debounces it live). */
      onchange: (value: string) => void;
    }
  | {
      kind: "source";
      /** The field's label. */
      label: string;
      sourceId: string;
      name: string;
      /** Screens with a layer bound to the source (to count the layers a change affects). */
      usedBy: readonly string[];
      /** After each save (the host refreshes its source list). */
      onsaved?: () => void;
    };

/** A list field linked to a whole source (binding column ""), as FieldInputs shows it. */
export interface LinkedList {
  sourceId: string;
  /** The source's name ("Linked to: <name>"). */
  name: string;
  /** Only data tables (text sources) open in the drawer; a schedule is edited on its own page. */
  kind: SourceKindName;
  usedBy: readonly string[];
  /** The binding in full ("Guests · all rows, sorted by Score ↓, top 10"), as a tooltip. */
  detail?: string;
  /** After each save from the drawer. */
  onsaved?: () => void;
}

/** A list field's limits (the server's). */
export const LIST_LIMITS: SheetLimits = { rows: LIST_ROWS_MAX, bytes: LIST_VALUE_MAX, cell: null };
/** A data table's limits (the server's). */
export const SOURCE_LIMITS: SheetLimits = { rows: SOURCE_ROWS_MAX, bytes: null, cell: SOURCE_CELL_MAX };

/** Layers of these screens bound to the source (any field). */
export function layersUsing(docs: readonly ScreenDoc[], sourceId: string): number {
  let n = 0;
  for (const d of docs) {
    for (const l of d.layers) {
      if (l.kind !== "media" && Object.values(l.bindings ?? {}).some((b) => b.sourceId === sourceId)) n++;
    }
  }
  return n;
}

export const affectsNote = (layers: number) => `Changes affect ${layers} ${layers === 1 ? "layer" : "layers"}.`;

/**
 * The current row after a sheet change: where the old current row went (`origin[i]` = the index
 * row `i` had before, -1 = new); deleted → the row now at its place (clamped); no rows → 0.
 */
export function followIndex(index: number, origin: readonly number[]): number {
  if (origin.length === 0) return 0;
  const at = origin.indexOf(index);
  if (at >= 0) return at;
  return Math.min(Math.max(0, index), origin.length - 1);
}

/**
 * The drawer's data table autosave: `touch()` after each change; the save starts `delayMs` after
 * the last one, one at a time (a change during a save saves again after it). `flush()` saves a
 * pending change now. A timer exists only while a change is pending.
 */
export class SourceSaver<D> {
  #timer: ReturnType<typeof setTimeout> | undefined;
  #dirty = false;
  #inFlight: Promise<void> | null = null;

  constructor(
    private readonly delayMs: number,
    private readonly snapshot: () => D,
    private readonly save: (doc: D) => Promise<unknown>,
    private readonly onstate: (s: "pending" | "saving" | "saved" | "failed", error?: unknown) => void,
  ) {}

  touch(): void {
    this.#dirty = true;
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => void this.flush(), this.delayMs);
    if (!this.#inFlight) this.onstate("pending");
  }

  flush(): Promise<void> {
    clearTimeout(this.#timer);
    this.#timer = undefined;
    if (this.#inFlight) return this.#inFlight;
    if (!this.#dirty) return Promise.resolve();
    this.#dirty = false;
    this.onstate("saving");
    const p = this.save(this.snapshot()).then(
      () => {
        this.#inFlight = null;
        if (this.#dirty) return this.flush();
        this.onstate("saved");
      },
      (e: unknown) => {
        this.#inFlight = null;
        this.#dirty = true;
        this.onstate("failed", e);
      },
    );
    this.#inFlight = p;
    return p;
  }
}

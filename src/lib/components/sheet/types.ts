// The spreadsheet editor's contract, shared by its hosts (Control's list drawer, the Sources
// page's Data table editor). The editor itself (SheetEditor.svelte) is a lazy chunk: hosts import
// it with a dynamic import() only.

import type { Snippet } from "svelte";
import type { SourceColumn } from "$lib/types";

/** One row: column key → cell text (image cells hold /assets/<name>). */
export type SheetRow = Record<string, string>;

export interface SheetLimits {
  /** Max rows (list fields: LIST_ROWS_MAX; data tables: 1000). */
  rows: number;
  /** Max bytes of the serialised rows (list fields: 64 KB); null = none. */
  bytes: number | null;
  /** Max bytes per cell (data tables: 4 KB); null = none. */
  cell: number | null;
  /** Max columns when `editableColumns` (data tables: 16); absent = SOURCE_COLUMNS_MAX. */
  columns?: number;
}

export interface SheetEditorProps {
  /** Shown in the drawer/page header. */
  title: string;
  /** Optional context line, e.g. "Changes affect 3 layers." */
  note?: string | null;
  columns: SourceColumn[];
  rows: SheetRow[];
  limits: SheetLimits;
  /** Column editing (label, key, kind, add/remove/reorder): data tables only. */
  editableColumns?: boolean;
  /** Every change (cells, rows, order, paste, undo/redo) with the full new rows (and columns when
   * editable). The host saves/applies it (debounced as it sees fit). `origin[i]` is the index
   * row `i` had before this change (-1 = a new row), so a host can follow a row (e.g. a data
   * table's current row) through moves and deletes. */
  onchange: (rows: SheetRow[], columns: SourceColumn[], origin: readonly number[]) => void;
  /** Esc on a non-editing grid (the drawer host closes; the page host ignores it). */
  onclose?: () => void;

  // ---- Additive (all optional) ----------------------------------------------------------------

  /** The host shows its own header: `title` only names the grid for assistive tech. */
  hideTitle?: boolean;
  /** A row marked as the current one (accent bar), e.g. a data table's current row. */
  currentRow?: number | null;
  /** Column keys that can't be edited (screens are bound to them); the label still can. */
  lockedKeys?: readonly string[];
  /** Host actions in the toolbar, given the active cell's row (null = none), e.g. Make current. */
  toolbar?: Snippet<[number | null]>;
  /** A read-only preview (e.g. an API source's mapped rows): no edits, paste, reorder or undo;
   * moving around and selecting rows still work. */
  readonly?: boolean;
}

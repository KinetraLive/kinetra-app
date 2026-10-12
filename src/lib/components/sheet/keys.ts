// The sheet's keyboard model: where the active cell goes for a key. Pure: no Svelte, no DOM.
//
//   Tab / Shift+Tab   along the row, wrapping to the next / previous row (stays at either end)
//   arrows            one cell, clamped; Ctrl+arrow to the edge
//   Home / End        first / last column of the row; with Ctrl the first / last row too
//   PageUp / PageDown `page` rows
//   Enter (commit)    one row down; on the last row it asks for a new row

export interface Cell {
  row: number;
  col: number;
}

export interface Size {
  rows: number;
  cols: number;
}

const clamp = (n: number, lo: number, hi: number) => Math.min(Math.max(n, lo), hi);

/** The cell clamped into the grid, or null when the grid has no cells. */
export function clampCell(at: Cell, size: Size): Cell | null {
  if (size.rows <= 0 || size.cols <= 0) return null;
  return { row: clamp(at.row, 0, size.rows - 1), col: clamp(at.col, 0, size.cols - 1) };
}

/** Tab (forward) or Shift+Tab (back): along the row, wrapping between rows; unchanged at either end. */
export function tab(at: Cell, back: boolean, size: Size): Cell {
  const lin = at.row * size.cols + at.col + (back ? -1 : 1);
  if (lin < 0 || lin >= size.rows * size.cols) return at;
  return { row: Math.floor(lin / size.cols), col: lin % size.cols };
}

export type MoveKey = "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" | "Home" | "End" | "PageUp" | "PageDown";

export const isMoveKey = (k: string): k is MoveKey =>
  k === "ArrowUp" ||
  k === "ArrowDown" ||
  k === "ArrowLeft" ||
  k === "ArrowRight" ||
  k === "Home" ||
  k === "End" ||
  k === "PageUp" ||
  k === "PageDown";

/** The cell a navigation key moves to (clamped; `ctrl` jumps to the edge). */
export function move(at: Cell, key: MoveKey, size: Size, ctrl = false, page = 20): Cell {
  const last = { row: Math.max(0, size.rows - 1), col: Math.max(0, size.cols - 1) };
  let { row, col } = at;
  switch (key) {
    case "ArrowUp":
      row = ctrl ? 0 : row - 1;
      break;
    case "ArrowDown":
      row = ctrl ? last.row : row + 1;
      break;
    case "ArrowLeft":
      col = ctrl ? 0 : col - 1;
      break;
    case "ArrowRight":
      col = ctrl ? last.col : col + 1;
      break;
    case "Home":
      col = 0;
      if (ctrl) row = 0;
      break;
    case "End":
      col = last.col;
      if (ctrl) row = last.row;
      break;
    case "PageUp":
      row -= page;
      break;
    case "PageDown":
      row += page;
      break;
  }
  return { row: clamp(row, 0, last.row), col: clamp(col, 0, last.col) };
}

/** Enter after an edit: one row down, or `addRow` on the last row (the new row's same column). */
export function enterDown(at: Cell, size: Size): { cell: Cell; addRow: boolean } {
  if (at.row >= size.rows - 1) return { cell: { row: size.rows, col: at.col }, addRow: true };
  return { cell: { row: at.row + 1, col: at.col }, addRow: false };
}

/** A key that starts editing with its character (printable, no Ctrl/Meta/Alt). */
export function isTypingKey(e: { key: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean }): boolean {
  return e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey;
}

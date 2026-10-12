// Undo / redo for the sheet: a capped stack of snapshots (the rows and columns are immutable
// arrays, so a snapshot is just references). `push` before a change; a new change clears redo.

export class History<T> {
  #undo: T[] = [];
  #redo: T[] = [];
  readonly cap: number;

  constructor(cap = 100) {
    this.cap = Math.max(1, cap);
  }

  get canUndo() {
    return this.#undo.length > 0;
  }

  get canRedo() {
    return this.#redo.length > 0;
  }

  get size() {
    return this.#undo.length;
  }

  /** Records the state before a change (the oldest snapshot goes past the cap). */
  push(before: T) {
    this.#undo.push(before);
    if (this.#undo.length > this.cap) this.#undo.shift();
    this.#redo = [];
  }

  /** Drops the last pushed snapshot (an edit that was cancelled). */
  pop(): T | undefined {
    return this.#undo.pop();
  }

  /** The state to go back to (null when there is none); `current` becomes redoable. */
  undo(current: T): T | null {
    const prev = this.#undo.pop();
    if (prev === undefined) return null;
    this.#redo.push(current);
    return prev;
  }

  /** The state to go forward to (null when there is none); `current` becomes undoable. */
  redo(current: T): T | null {
    const next = this.#redo.pop();
    if (next === undefined) return null;
    this.#undo.push(current);
    if (this.#undo.length > this.cap) this.#undo.shift();
    return next;
  }

  clear() {
    this.#undo = [];
    this.#redo = [];
  }
}

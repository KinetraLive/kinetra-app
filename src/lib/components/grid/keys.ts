// Keyboard movement in CSS grids of cards (templates list, gallery, screens list). Pure.

/** Index after a grid arrow key, or null when the key isn't a move. */
export function moveIndex(key: string, index: number, count: number, columns: number): number | null {
  if (count === 0) return null;
  const cols = Math.max(1, columns);
  let next: number;
  switch (key) {
    case "ArrowLeft":
      next = index - 1;
      break;
    case "ArrowRight":
      next = index + 1;
      break;
    case "ArrowUp":
      next = index - cols;
      break;
    case "ArrowDown":
      next = index + cols;
      break;
    case "Home":
      next = 0;
      break;
    case "End":
      next = count - 1;
      break;
    default:
      return null;
  }
  return Math.min(count - 1, Math.max(0, next));
}

/** Number of columns a CSS grid currently lays out (1 when unknown, e.g. in jsdom). */
export function gridColumns(el: Element | undefined): number {
  if (!el || typeof getComputedStyle === "undefined") return 1;
  const cols = getComputedStyle(el).gridTemplateColumns;
  return cols && cols !== "none" ? cols.trim().split(/\s+/).length : 1;
}

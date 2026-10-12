// Control's resizable right panel: the width range (mirrors CONTROL_PANEL_WIDTH_RANGE in
// model.rs) and the divider's keyboard steps. Pure.

export const PANEL_WIDTH = { min: 280, max: 640, step: 16 } as const;

export const clampPanel = (w: number) => Math.round(Math.min(PANEL_WIDTH.max, Math.max(PANEL_WIDTH.min, w)));

/**
 * The width after a key on the divider, or null for other keys. The divider sits left of the
 * panel: ← moves it left (a wider panel), → narrower; Home / End go to the smallest / largest.
 */
export function keyWidth(key: string, w: number): number | null {
  switch (key) {
    case "ArrowLeft":
      return clampPanel(w + PANEL_WIDTH.step);
    case "ArrowRight":
      return clampPanel(w - PANEL_WIDTH.step);
    case "Home":
      return PANEL_WIDTH.min;
    case "End":
      return PANEL_WIDTH.max;
    default:
      return null;
  }
}

/** The width while dragging: the start width plus how far the pointer moved left. */
export const dragWidth = (startWidth: number, startX: number, x: number) => clampPanel(startWidth + (startX - x));

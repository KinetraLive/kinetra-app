// Opens the version history in the main window from anywhere (the chip, the palette). The top bar
// hosts the lazy dialog and registers how to open it; nothing else is loaded until it opens.
// Plain functions (no runes): this is in every page's shell.

let opener: (() => void) | null = null;

/** The top bar registers its opener (null on teardown). */
export function setHistoryOpener(open: (() => void) | null): void {
  opener = open;
}

/** Opens the version history (no-op outside the main window's shell). */
export function openHistory(): void {
  opener?.();
}

/** The dialog component (its own lazy chunk). */
export const loadHistory = () => import("./HistoryDialog.svelte").then((m) => m.default);

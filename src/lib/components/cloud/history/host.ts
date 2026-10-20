// Opens the version history in the main window from anywhere (the chip, the palette). The top bar
// hosts the lazy dialog and registers how to open it; nothing else is loaded until it opens.
// Plain functions (no runes): this is in every page's shell.

type Opener = (restore?: string) => void;
let opener: Opener | null = null;
let ready = false;

/** The top bar's cloud part says whether the open project has a backup to show. */
export function setHistoryReady(on: boolean): void {
  ready = on;
}

/** Whether "Version history" has anything to show (the palette asks). */
export function historyReady(): boolean {
  return ready;
}

/** The top bar registers its opener (null on teardown). */
export function setHistoryOpener(open: Opener | null): void {
  opener = open;
}

/** Opens the version history (no-op outside the main window's shell); `restore`: a version to offer
 * restoring at once ("Open the other version"). */
export function openHistory(restore?: string): void {
  opener?.(restore);
}

/** The dialog component (its own lazy chunk). */
export const loadHistory = () => import("./HistoryDialog.svelte").then((m) => m.default);

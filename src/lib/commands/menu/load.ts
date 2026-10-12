// The one hook into the native menu (main window, after first paint; called by stores/windows.ts,
// the root layout's lazy window chunk): a module with no static imports, so that chunk carries only
// this import and not the menu chunk's preload list. The sync runs for the window's lifetime.

let started = false;

export function loadMenu(): Promise<void> {
  if (started) return Promise.resolve();
  started = true;
  return import("./sync.svelte").then(
    (m) => void m.startMenuSync(),
    () => void (started = false),
  );
}

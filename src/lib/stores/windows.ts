// Keeps this window's copies of prefs, UI state and the open project in step with the other app
// window (main ⇄ Preferences). The backend sends each change to every app window except the one
// whose command caused it, so applying it with `set` never echoes an optimistic write back.
// Also the root layout's other window chores (`startWindow`): showing the window after the first
// paint and loading the main window's native menu. One lazy chunk, loaded at mount (the picker's
// budget: one dynamic import in the layout instead of three).

import { on, toAppError } from "$lib/api";
import { loadMenu } from "$lib/commands/menu/load";
import { loadLicense } from "$lib/components/license/load";
import { prefs } from "./prefs.svelte";
import { project, uiState } from "./project.svelte";
import { toasts } from "./toasts.svelte";

export function syncWindows(): () => void {
  const offs = [
    on("prefs-changed", (p) => prefs.set(p)),
    on("ui-state-changed", (u) => uiState.set(u)),
    on("project-changed", (p) => {
      // Preferences: settings, move, import, reset; main: open, close. The UI state arrives as
      // `ui-state-changed`; it is reloaded here too when the project itself changed, whatever
      // order the events come in.
      const prev = project.current;
      project.current = p;
      if (!p) uiState.set(null);
      else if (!prev || prev.path !== p.path)
        uiState.load().catch((e: unknown) => {
          toasts.push({ kind: "error", message: `Couldn't load the project's layout: ${toAppError(e).message}` });
        });
    }),
  ];
  return () => {
    for (const off of offs) off();
  };
}

/**
 * The root layout's window setup (every window, at mount): starts the cross-window sync and
 * requests the window module. `show` (after the first paint) shows the window and, in the main
 * window, loads the native menu (lazy, for the app's lifetime). Errors (e.g. a plain browser) are
 * ignored.
 */
export function startWindow(): { stop: () => void; show: () => void } {
  const stop = syncWindows();
  const windowApi = import("@tauri-apps/api/window");
  windowApi.catch(() => {});
  return {
    stop,
    show: () =>
      void windowApi
        .then(({ getCurrentWindow }) => {
          const w = getCurrentWindow();
          if (w.label === "main") {
            void loadMenu();
            // Licensing: welcome screen, sign-in, notices (a lazy chunk on <body>).
            void loadLicense();
          }
          return w.show();
        })
        .catch(() => {}),
  };
}

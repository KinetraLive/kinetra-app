// The one hook into the license UI (main window, after first paint; called by stores/windows.ts):
// no static imports, so the window chunk carries only this import. Mounts the license host on
// <body> for the app's lifetime: welcome screen, sign-in, notices, Move Pro / seats dialogs and
// the Beta Ended screen.

let started = false;

export function loadLicense(): Promise<void> {
  if (started) return Promise.resolve();
  started = true;
  return Promise.all([import("svelte"), import("./LicenseHost.svelte"), import("./state.svelte")]).then(
    ([{ mount }, host, s]) => {
      s.license.start();
      mount(host.default, { target: document.body });
    },
    () => void (started = false),
  );
}

<!--
  Root layout (every window: main and Preferences): global CSS, theme sync, prefs load, the
  cross-window store sync, backend notices -> toasts, and showing the (initially hidden) window
  after the first paint. ToastHost is mounted here once.
-->
<script lang="ts">
  import "../app.css";
  import { onMount, type Component, type Snippet } from "svelte";
  import { on, toAppError } from "$lib/api";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { theme } from "$lib/stores/theme.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";

  let { children }: { children: Snippet } = $props();

  // ToastHost (and its transitions) stays out of the initial bundle: it is preloaded when the
  // browser is idle after first paint, or immediately if a toast arrives before that. Loading it
  // early matters for screen readers: its aria-live region should exist before toasts land in it.
  let ToastHost = $state<Component | null>(null);
  let hostLoading = false;
  function loadToastHost() {
    if (hostLoading) return;
    hostLoading = true;
    import("$lib/components/toast-host")
      .then((m) => m.loadToastHost())
      .then((m) => (ToastHost = m.default))
      .catch(() => (hostLoading = false));
  }
  $effect(() => {
    if (toasts.list.length > 0) loadToastHost();
  });

  onMount(() => {
    const idle = window.requestIdleCallback ?? ((cb: () => void) => setTimeout(cb, 200));
    idle(loadToastHost);

    const stopTheme = theme.init();
    let cancelled = false;
    // Main ⇄ Preferences window: prefs, UI state and the project follow the other window's
    // changes. The same lazy chunk requests the window module now and shows the window later.
    let stopSync = () => {};
    const win = import("$lib/stores/windows").then((m) => {
      const w = m.startWindow();
      if (cancelled) w.stop();
      else stopSync = w.stop;
      return w;
    });
    win.catch(() => {});

    prefs.load().catch((e: unknown) => {
      toasts.push({
        kind: "error",
        message: `Couldn't load preferences: ${toAppError(e).message}`,
      });
    });

    const offRecovered = on("store-recovered", ({ file, quarantinedAs }) => {
      toasts.push({
        kind: "error",
        message: `${file} was damaged and has been reset to defaults. The damaged copy was moved to ${quarantinedAs}.`,
      });
    });
    const offBackground = on("background-error", ({ message }) => {
      toasts.push({ kind: "error", message });
    });

    // Show the window only once the UI font is ready and a frame (with the right
    // theme) has painted, so there is no font swap flash. Font loading is capped at 300 ms: the
    // files are local, and a slow disk must never keep the window hidden. The window module is
    // loaded lazily (~8 KB gz, by stores/windows.ts) but requested now. Errors (e.g. a plain
    // browser) are ignored. The main window also loads the native menu then.
    const fontsReady = Promise.race([
      document.fonts.load('400 13px "IBM Plex Sans"').catch(() => {}),
      new Promise((r) => setTimeout(r, 300)),
    ]);
    let raf = 0;
    fontsReady.then(() => {
      if (cancelled) return;
      raf = requestAnimationFrame(() => {
        raf = requestAnimationFrame(() => {
          win.then((w) => w.show(), () => {});
        });
      });
    });

    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
      offRecovered();
      offBackground();
      stopTheme();
      stopSync();
    };
  });
</script>

{@render children()}
{#if ToastHost}<ToastHost />{/if}

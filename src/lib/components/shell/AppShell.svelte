<!--
  The app frame. A grid: the top bar across the full width, below it the sidebar and the content.
  Panels are flush and separated by 1px lines; chrome (top bar, rail) on --surface-1, content on
  --bg. The sidebar column is --rail-w (icon rail; its hover overlay floats above the content, so
  the content never reflows) or --sidebar-w when pinned.

  Owns the shell's ONE keydown listener, on window in the capture phase so it runs before any
  element (CodeMirror included): every key goes through the command registry and the binding table
  ($lib/commands: runShortcut). A key that ran a command (or hit a conflict) is consumed: default
  prevented and propagation stopped, so an editor never also acts on it. Nothing runs while the
  palette is open (it handles its own keys, Esc included). Ctrl+K / Ctrl+P / Ctrl+Shift+P are the
  default keys of "palette.open", which opens the command palette (a lazy chunk, loaded on first
  open). Registers the shell's commands (navigation, theme, sidebar pin, Automation off / on, Apply
  style…) and
  provides the command context's `route` and `project`. Shows `flow-log` toasts (a Log action's "Toast") on every
  page, and `control-conflict` toasts (a MIDI input assigned twice ran nothing; links to Preferences >
  MIDI & OSC). While Automation is off (ui-state `control.automationOff`) an amber "Automation off" pill
  sits in the middle of the top bar, linking to Control. On a route change the content scrolls
  back to the top and the page slides in (entry only: --page-shift + fade over --dur, a
  compositor-only Web Animation that never blocks input; zero under reduced motion). The page goes
  through SimpleGate (Simple mode shows a hint instead of /flows).

  Props:
    children?: Snippet   the routed page
-->
<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import { afterNavigate, goto } from "$app/navigation";
  import { page } from "$app/state";
  import { on, openPreferences, toAppError } from "$lib/api";
  import {
    provideContext,
    registerCommands,
    routeOf,
    runShortcut,
    setFollowUpOpener,
  } from "$lib/commands/registry.svelte";
  import { conflictText } from "$lib/components/midi/source";
  import { uiMode } from "$lib/stores/mode.svelte";
  import { project } from "$lib/stores/project.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { uiState } from "$lib/stores/uiState.svelte";
  import { shellCommands } from "./commands";
  import Sidebar from "./Sidebar.svelte";
  import SimpleGate from "./SimpleGate.svelte";
  import TopBar from "./TopBar.svelte";
  import { remote } from "$lib/components/remote/state.svelte";
  import { setRemote } from "$remote";
  import { cloud } from "$lib/components/cloud/state.svelte";
  import { openHistory } from "$lib/components/cloud/history/host";

  let { children }: { children?: Snippet } = $props();

  let content: HTMLElement | undefined = $state();
  let pageEl: HTMLElement | undefined = $state();
  const pinned = $derived(uiState.value?.sidebarPinned ?? false);
  const automationOff = $derived(uiState.value?.control.automationOff ?? false);

  async function toggleAutomation() {
    const v = uiState.value;
    if (v) await uiState.update({ ...v, control: { ...v.control, automationOff: !v.control.automationOff } });
  }

  /** Remote control on / off (the palette); refusals (Pro, sign-in) are a toast. */
  async function toggleRemote() {
    if (!setRemote) return;
    try {
      await setRemote(!(remote.view?.on ?? false));
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    }
  }

  type PaletteT = typeof import("$lib/commands/palette/Palette.svelte").default;
  let Palette = $state<PaletteT | null>(null);
  let paletteOpen = $state(false);
  /** Open straight into this command's follow-up step (its key was pressed). */
  let followUp = $state<string | undefined>(undefined);

  async function openPalette(step?: string) {
    if (paletteOpen) return;
    try {
      Palette ??= (await import("$lib/commands/palette/Palette.svelte")).default;
      followUp = step;
      paletteOpen = true;
    } catch (e) {
      commandFailed(e);
    }
  }

  function commandFailed(e: unknown) {
    toasts.push({ kind: "error", message: toAppError(e).message });
  }

  // Capture phase on window: before CodeMirror or any page handler sees the key.
  onMount(() => {
    const onkeydown = (e: KeyboardEvent) => {
      if (e.defaultPrevented || paletteOpen) return;
      if (runShortcut(e)) {
        e.preventDefault();
        e.stopPropagation();
      }
    };
    window.addEventListener("keydown", onkeydown, true);
    return () => window.removeEventListener("keydown", onkeydown, true);
  });

  $effect(() => provideContext({ route: routeOf(page.url.pathname), project: project.current !== null }));

  // A key bound to a command with a follow-up step opens the palette (it shows the step).
  onMount(() => {
    setFollowUpOpener((id) => void openPalette(id));
    return () => setFollowUpOpener(null);
  });

  // A flow's Log action with "Toast" on: shown on every page (the server throttles it).
  // Simple / Advanced on <html>: `.advanced-only` (app.css) hides small links in Simple mode for free.
  $effect(() => {
    document.documentElement.dataset.uiMode = uiMode();
  });

  onMount(() => on("flow-log", (l) => toasts.push({ kind: "info", message: l.message })));
  // A MIDI input assigned to several actions ran nothing (the core sends it ≤ 1 per 2 s per source).
  onMount(() =>
    on("control-conflict", (c) =>
      toasts.push({
        kind: "error",
        message: conflictText(c.source, c.mappingIds.length),
        timeoutMs: 6000,
        action: { label: "MIDI & OSC", run: () => void openPreferences("midi").catch(commandFailed) },
      }),
    ),
  );

  onMount(() =>
    registerCommands(
      shellCommands({
        openPalette: () => void openPalette(),
        go: (href) => void goto(href),
        toggleAutomation: () => void toggleAutomation(),
        automationOff: () => automationOff,
        toggleRemote: setRemote ? () => void toggleRemote() : undefined,
        remoteOn: () => remote.view?.on ?? false,
        openHistory,
        historyReady: () => !!cloud.view?.project?.registered,
      }),
    ),
  );

  /** "160ms" / "0.16s" → 160. */
  function ms(v: string): number {
    const n = parseFloat(v);
    if (!Number.isFinite(n)) return 0;
    return v.trim().endsWith("ms") ? n : n * 1000;
  }

  /** Entry-only page transition: --page-shift slide up + fade, --dur, --ease. */
  function enterPage(el: HTMLElement) {
    if (typeof el.animate !== "function") return;
    const css = getComputedStyle(el);
    const duration = ms(css.getPropertyValue("--dur"));
    if (duration <= 0) return;
    const shift = css.getPropertyValue("--page-shift").trim() || "0px";
    el.animate(
      [
        { opacity: 0, transform: `translateY(${shift})` },
        { opacity: 1, transform: "none" },
      ],
      { duration, easing: css.getPropertyValue("--ease").trim() || "ease-out" },
    );
  }

  // The content pane scrolls, not the window, so SvelteKit's scroll reset doesn't reach it.
  afterNavigate(({ from, to }) => {
    if (from?.url.pathname === to?.url.pathname) return;
    if (content) content.scrollTop = 0;
    if (pageEl) enterPage(pageEl);
  });
</script>

<div class="shell" data-sidebar={pinned ? "pinned" : "rail"}>
  <div class="top">
    <TopBar />
    {#if automationOff}<a class="auto-off vf" href="/control">Automation off</a>{/if}
  </div>
  <Sidebar />
  <main class="content" bind:this={content}>
    <!-- .page clips vertically; only .page-inner moves, so the entry slide never changes the
         scrollable height (a moving min-height:100% box briefly overflowed by --page-shift, a
         scrollbar appeared for one animation and the whole page jumped sideways). -->
    <div class="page">
      <div class="page-inner" bind:this={pageEl}>
        <SimpleGate path={page.url.pathname}>{@render children?.()}</SimpleGate>
      </div>
    </div>
  </main>
</div>

{#if Palette && paletteOpen}
  <Palette onclose={() => (paletteOpen = false)} {followUp} />
{/if}

<style>
  .shell {
    display: grid;
    grid-template-columns: var(--rail-w) minmax(0, 1fr);
    grid-template-rows: var(--topbar-h) minmax(0, 1fr);
    height: 100%;
    background: var(--bg);
  }
  /* Pinning changes the column instantly (no width animation); hover expansion never does. */
  .shell[data-sidebar="pinned"] {
    grid-template-columns: var(--sidebar-w) minmax(0, 1fr);
  }
  .top {
    display: grid;
    grid-column: 1 / -1;
    min-width: 0;
    /* Above the sidebar overlay and the content, so the project menu overlaps both. */
    position: relative;
    z-index: 4;
  }
  .auto-off {
    --vf-radius: var(--r-full);
    position: absolute;
    top: 50%;
    left: 50%;
    padding: 2px var(--s-3);
    border-radius: var(--r-full);
    box-shadow: inset 0 0 0 1px var(--warning);
    color: var(--warning);
    font-size: var(--fs-sm);
    font-weight: 600;
    text-decoration: none;
    transform: translate(-50%, -50%);
    white-space: nowrap;
  }
  .content {
    min-width: 0;
    overflow: auto;
    overscroll-behavior: contain;
    /* Reserve the scrollbar's space so pages that scroll and pages that don't have the same
       width: navigating between them never shifts the layout sideways. */
    scrollbar-gutter: stable;
    background: var(--bg);
  }
  .page {
    display: grid;
    min-height: 100%;
    /* The entry animation moves .page-inner down by --page-shift; clip that instead of letting
       it grow the scroll area. Horizontal overflow stays visible. */
    overflow-x: visible;
    overflow-y: clip;
  }
  .page-inner {
    min-width: 0;
  }
</style>

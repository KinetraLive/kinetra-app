<!--
  Control: the operator view. Full width, pro-tool density.
    toolbar   view, columns, screens shown, live edit, Clear all, Automation off, PANIC (ControlToolbar)
    outputs   display outputs showing and their warnings (a lazy chunk, only while outputs exist)
    main      grid view: live tiles of the visible screens (≤ 6 iframes, the selected one first);
              single view: a screen list and ONE large LivePreview of the selected screen
    right     the selected screen, in operator terms: LAYERS (TAKE / CLEAR, fields, timers, hotkeys)
              and its sources strip (Prev / Next, auto-advance); CUES aimed at it (Run / Stop,
              hotkeys; "Show all cues" for the whole project); AUTOMATION (a lazy chunk: one row
              per installed recipe of this screen with its operator panel and rules, the screen's
              other rules, and the recipes that need attention); "Manage in Flows →" at the bottom

  Hotkeys (this page only): F1–F12 (+Shift) toggle the mapped layer or run the mapped cue (again =
  restart, or ignored, per flow), Shift+Escape is PANIC; layer and cue keys are ignored while typing
  and on repeat. They are commands registered while the page is mounted (control/commands.svelte.ts:
  "live.toggle:<screen>/<layer>", "cue.go:<flow>", "live.panic", key scope "control"), run by the
  shell's keydown through the binding table. The page provides the command context's screen and
  Studio mode. All state and actions live in ControlCtl. The linear flow editor and the
  auto-advance settings popover are lazy chunks.

  Studio mode (Preview / Program, ControlCtl.studio): the single view becomes the studio stage (a
  lazy chunk: a screen picker and a multiview, Preview and Program sized to the stage with TAKE /
  transition / T-bar between them; the screen list and side panel narrow to make room);
  layer rows cue layers into Preview; grid tiles say "Preview pending"; Space / Enter take the
  selected screen's Preview; turning it off with a pending Preview asks Take / Keep / Discard (a
  lazy dialog).

  The right panel is resizable: a divider between stage and panel (PanelDivider: drag, or ← / →,
  Home / End), 280–640 px, saved per project (`control.panelWidth`) on release; while dragging only
  the `--panel-w` custom property changes, once per frame. Below 900 px the layout stacks and the
  divider goes away.

  Performance: hidden screens render nothing (no iframe); grid iframes follow LiveThumbs; the
  single view has exactly one preview (two in Studio mode: Preview and Program); nothing runs at
  rest (edits debounce only while typing).
-->
<script lang="ts">
  import { untrack } from "svelte";
  import Monitor from "lucide-svelte/icons/monitor";
  import Plus from "lucide-svelte/icons/plus";
  import Button from "$lib/components/Button.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import LivePreview from "$lib/components/LivePreview.svelte";
  import ControlGrid from "$lib/components/control/ControlGrid.svelte";
  import ControlLayers from "$lib/components/control/ControlLayers.svelte";
  import ControlToolbar from "$lib/components/control/ControlToolbar.svelte";
  import CuesStrip from "$lib/components/control/CuesStrip.svelte";
  import { lazy } from "$lib/components/control/LayerRow.svelte";
  import LazyStrip from "$lib/components/control/LazyStrip.svelte";
  import ScreenList from "$lib/components/control/ScreenList.svelte";
  import { ControlCtl } from "$lib/components/control/control.svelte";
  import { loadAutomation } from "$lib/components/control/strips";
  import { cueLabel } from "$lib/components/flows/flows.svelte";
  import { NewScreenLauncher } from "$lib/components/screens/new/launcher.svelte";
  import { isTyping } from "$lib/components/shell/nav";
  import { provideContext, registerCommands } from "$lib/commands/registry.svelte";
  import { live } from "$lib/stores/live.svelte";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { screenUrl, server } from "$lib/stores/server.svelte";

  // The panel divider is its own chunk (requested at once; a plain line shows until it arrives).
  const dividerChunk = import("$lib/components/control/PanelDivider.svelte");

  const ctl = new ControlCtl(isTyping);
  const newScreen = new NewScreenLauncher();

  $effect(() => ctl.connect());
  $effect(() => void ctl.loadDoc(ctl.selectedId));

  const sel = $derived(ctl.selected);

  let body: HTMLDivElement | undefined = $state();
  let side: HTMLElement | undefined = $state();
  const panelWidth = $derived(ctl.ui.panelWidth ?? null);

  /** Saves a dragged width; afterwards the stored one (or the default) shows, even on failure. */
  async function commitPanel(w: number) {
    await ctl.setPanelWidth(w);
    const saved = ctl.ui.panelWidth;
    if (saved) body?.style.setProperty("--panel-w", `${saved}px`);
    else body?.style.removeProperty("--panel-w");
  }
  const studio = ctl.studio;

  // Studio mode: its chunk (stage, palette commands) and the screens' Previews, only while on.
  $effect(() => {
    if (studio.on) void studio.loadUi();
  });
  $effect(() => studio.ui?.previews.watch());
  $effect(() => {
    if (studio.on) void studio.ui?.previews.load(ctl.screens.map((s) => s.screen.id));
  });
  $effect(() => {
    const title = studio.on ? "Studio mode off" : "Studio mode on";
    return untrack(() =>
      registerCommands([{ id: "studio.toggle", title, section: "settings", keyScope: "control", run: studio.toggle }]),
    );
  });
  // The hotkey commands are a chunk of their own, requested at once (keys work once it is in).
  $effect(() => {
    let off = () => {};
    let alive = true;
    void import("$lib/components/control/commands.svelte").then((m) => {
      if (alive) off = untrack(() => m.watchControlCommands(ctl, cueLabel));
    });
    return () => {
      alive = false;
      off();
    };
  });
  $effect(() => {
    const patch = { screenId: ctl.selectedId, studio: studio.on };
    return untrack(() => provideContext(patch));
  });
  $effect(() => {
    const ui = studio.ui;
    if (studio.on && ui) return ui.registerStudioCommands(ctl);
  });
  const pending = $derived.by(() => {
    const views = studio.on ? studio.ui?.previews.views : undefined;
    return new Set(views ? [...views.values()].filter((v) => v.cues.length > 0).map((v) => v.screenId) : []);
  });
</script>

<svelte:head><title>Control · Kinetra</title></svelte:head>
<svelte:window onkeydown={ctl.keydown} />

{#snippet stopped()}Server not running{/snippet}

<div class="control">
  <ControlToolbar {ctl} />
  {#if prefs.value?.outputs?.length}
    {#await import("$lib/components/control/OutputsStrip.svelte") then m}<m.default {ctl} />{/await}
  {/if}
  <!-- Test poll data: the question before Program and the notice when a remote take was refused
       (lazy; only while the project has test and real polls). -->
  {#if ctl.realPolls && ctl.testPolls.size > 0}
    {#await import("$lib/components/control/TestDataConfirm.svelte") then m}<m.default {ctl} />{/await}
  {/if}

  {#if live.loaded && ctl.screens.length === 0}
    <div class="center">
      <EmptyState
        title="Create your first screen"
        description="One browser source in OBS or vMix."
      >
        {#snippet icon()}<Monitor size={20} />{/snippet}
        {#snippet actions()}
          <Button variant="primary" onclick={newScreen.show}>
            {#snippet icon()}<Plus size={14} />{/snippet}
            New screen
          </Button>
        {/snippet}
      </EmptyState>
    </div>
  {:else if live.loaded && ctl.visible.length === 0}
    <div class="center">
      <EmptyState title="Every screen is hidden" description="Hidden screens stay on air.">
        {#snippet icon()}<Monitor size={20} />{/snippet}
        {#snippet actions()}
          <Button variant="primary" onclick={ctl.showAll}>Show screens</Button>
        {/snippet}
      </EmptyState>
    </div>
  {:else if ctl.visible.length > 0}
    <div
      class="body"
      class:studio={studio.on && ctl.ui.view === "single"}
      bind:this={body}
      style:--panel-w={panelWidth ? `${panelWidth}px` : undefined}
    >
      {#if ctl.ui.view === "grid"}
        <div class="main scroll">
          <ControlGrid
            screens={ctl.visible}
            columns={ctl.ui.gridColumns}
            selectedId={ctl.selectedId}
            status={server.status}
            {pending}
            onselect={ctl.select}
            onclear={ctl.clearScreen}
          />
        </div>
      {:else}
        <div class="main single" class:studio={studio.on}>
          {#if !studio.on}
            <nav class="list scroll" aria-label="Screens">
              <ScreenList screens={ctl.visible} selectedId={ctl.selectedId} onselect={ctl.select} />
            </nav>
          {/if}
          <div class="stage scroll">
            {#if sel && studio.on}
              {#if studio.ui}<studio.ui.StudioStage {ctl} view={sel} />{/if}
            {:else if sel}
              <LivePreview
                src={screenUrl(server.status, sel.screen.id)}
                title="{sel.screen.name} preview"
                width={sel.screen.width}
                height={sel.screen.height}
                children={server.status?.state === "running" ? undefined : stopped}
              />
            {/if}
          </div>
        </div>
      {/if}
      {#await dividerChunk}
        <div class="line"></div>
      {:then { default: PanelDivider }}
        <PanelDivider target={body} panel={side} width={panelWidth} controls="control-side" oncommit={(w) => void commitPanel(w)} />
      {/await}
      <!-- Right-click / Shift+F10 on a layer row or cue: its context menu (Assign MIDI…, lazy). -->
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <aside
        class="side scroll"
        id="control-side"
        aria-label="Selected screen"
        bind:this={side}
        oncontextmenu={(e) => lazy.body?.sideMenu(e, ctl)}
        onkeydown={(e) => lazy.body?.sideMenu(e, ctl)}
      >
        {#if sel}<ControlLayers {ctl} view={sel} />{/if}
        <CuesStrip {ctl} />
        <LazyStrip label="Automation" load={loadAutomation} {ctl} />
        <a class="manage advanced-only" href="/flows">Manage in Flows →</a>
      </aside>
    </div>
  {/if}
</div>

{#if newScreen.component}
  {@const NewScreen = newScreen.component}
  <NewScreen open={newScreen.open} onclose={newScreen.close} names={ctl.screens.map((s) => s.screen.name)} />
{/if}

{#if studio.asking && studio.ui}
  <studio.ui.StudioExit screens={studio.asking} onchoose={studio.exit} />
{/if}

{#if ctl.flowEditor.component && ctl.flowEditor.target}
  {@const Editor = ctl.flowEditor.component}
  {#key ctl.flowEditor.target}
    <Editor target={ctl.flowEditor.target} {ctl} onclose={ctl.flowEditor.close} />
  {/key}
{/if}

<style>
  .control {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    height: calc(100dvh - var(--topbar-h));
    min-width: 0;
  }
  .center {
    display: grid;
    flex: 1;
    place-items: center;
    padding: var(--s-6);
  }
  .body {
    display: grid;
    flex: 1;
    /* The panel: dragged (--panel-w, always leaving the stage 320px) or the default. */
    grid-template-columns: minmax(0, 1fr) 1px var(--panel-w-or, minmax(20rem, 28rem));
    min-height: 0;
  }
  .body[style*="--panel-w"] {
    --panel-w-or: min(var(--panel-w), calc(100cqw - 321px));
  }
  .scroll {
    min-height: 0;
    overflow: auto;
  }
  .main {
    min-width: 0;
    padding: var(--s-3);
  }
  .single {
    display: grid;
    grid-template-columns: minmax(12rem, 16rem) minmax(0, 1fr);
    padding: 0;
  }
  /* Studio mode: two outputs side by side need the width. */
  .single.studio {
    grid-template-columns: minmax(0, 1fr);
  }
  .body.studio {
    grid-template-columns: minmax(0, 1fr) 1px var(--panel-w-or, minmax(20rem, 24rem));
  }
  .single.studio .stage {
    padding: var(--s-3);
  }
  .list {
    border-right: 1px solid var(--line);
    background: var(--surface-1);
  }
  .stage {
    padding: var(--s-4);
  }
  .line {
    background: var(--line);
  }
  .side {
    min-width: 0;
    background: var(--surface-1);
  }
  .manage {
    display: block;
    padding: var(--s-2) var(--s-3) var(--s-4);
    border-top: 1px solid var(--line);
    color: var(--accent-text);
    font-size: var(--fs-sm);
  }
  @container (max-width: 900px) {
    .control {
      height: auto;
    }
    .body,
    .body.studio {
      grid-template-columns: minmax(0, 1fr);
    }
    .body > :global(.divider),
    .line {
      display: none;
    }
    .side {
      border-top: 1px solid var(--line);
    }
  }
</style>

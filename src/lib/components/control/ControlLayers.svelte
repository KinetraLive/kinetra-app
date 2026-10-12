<!--
  ControlLayers: the right panel for the selected screen. Header (screen name, Clear all for this
  screen), one LayerRow per layer front first (the reverse of the doc order), then the sources
  strip (Prev / Next, auto-advance; a lazy chunk, loaded the first time a selected screen has bound
  sources) when the screen's layers are bound to sources. Without layers: "Open layout editor".
  The Cues and Automation strips sit under this panel (the page renders them).

  Studio mode: "Take preview" in the header (the grid view has no studio column).

  Props:
    ctl: ControlCtl
    view: LiveScreenView   the selected screen
-->
<script lang="ts">
  import type { Component } from "svelte";
  import PencilRuler from "lucide-svelte/icons/pencil-ruler";
  import Button from "$lib/components/Button.svelte";
  import type { LiveScreenView } from "$lib/types";
  import { onAirCount, type ControlCtl } from "./control.svelte";
  import LayerRow, { lazy } from "./LayerRow.svelte";
  import { loadSourcesStrip, type SourcesStripProps } from "./strips";

  let { ctl, view }: { ctl: ControlCtl; view: LiveScreenView } = $props();

  const uid = $props.id();
  const id = $derived(view.screen.id);
  const rows = $derived([...view.layers].reverse());
  const count = $derived(onAirCount(view));
  const bound = $derived(ctl.screenSources.length > 0);

  // While a layer here has several variants: "Switch <layer> to <variant>" in the palette (through
  // the rows' lazy chunk).
  const variants = $derived(rows.some((l) => (ctl.templateOf(l.id)?.variantCount ?? 0) > 1));
  $effect(() => {
    const body = lazy.body;
    if (variants && body) return body.registerVariantCommands(ctl, id);
  });
  // MIDI mappings (the rows' and cues' MIDI chips) while Control is open (lazy).
  $effect(() => lazy.body?.watchControls());

  let SourcesStrip = $state.raw<Component<SourcesStripProps> | null>(null);
  $effect(() => {
    if (!bound || SourcesStrip) return;
    let alive = true;
    loadSourcesStrip().then(
      (c) => alive && (SourcesStrip = c),
      () => {},
    );
    return () => {
      alive = false;
    };
  });
</script>

<section class="panel" aria-labelledby="{uid}-title">
  <header class="head">
    <h2 class="title" id="{uid}-title">{view.screen.name}</h2>
    <span class="res mono">{view.screen.width}×{view.screen.height}</span>
    <span class="spacer"></span>
    {#if ctl.studio.on}
      <Button
        size="sm"
        variant="primary"
        disabled={!ctl.studio.ui?.previews.pending(id)}
        onclick={() => ctl.studio.take(id)}
        aria-label="Take the Preview of {view.screen.name}"
      >
        Take preview
      </Button>
    {/if}
    <Button size="sm" variant="ghost" href="/screens/{id}" aria-label="Open layout editor for {view.screen.name}">
      {#snippet icon()}<PencilRuler size={14} />{/snippet}
      Layout
    </Button>
  </header>

  {#if rows.length === 0}
    <div class="empty">
      <p>No layers on this screen yet.</p>
      <Button size="sm" variant="primary" href="/screens/{id}">
        {#snippet icon()}<PencilRuler size={14} />{/snippet}
        Open layout editor
      </Button>
    </div>
  {:else}
    <div class="sub">
      <span class="section-label">Layers</span>
      <span class="count">{count} on air</span>
    </div>
    <ul class="rows" aria-label="Layers of {view.screen.name}, front to back">
      {#each rows as layer (layer.id)}
        <LayerRow {ctl} screenId={id} {layer} />
      {/each}
    </ul>
  {/if}

  {#if bound && SourcesStrip}
    <SourcesStrip
      sources={ctl.screenSources}
      onstep={ctl.step}
      autoOf={(s) => ctl.autoOf(s)}
      onauto={ctl.setAutoOn}
      onautosettings={ctl.setAutoSettings}
      onapi={ctl.putSource}
    />
  {/if}
</section>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: calc(var(--row-h) + var(--s-2));
    padding: var(--s-1) var(--s-2) var(--s-1) var(--s-3);
    border-bottom: 1px solid var(--line);
  }
  .title {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    font-size: var(--fs-md);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .res,
  .count {
    font-variant-numeric: tabular-nums;
    flex: none;
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .spacer {
    flex: 1;
  }
  .sub {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--s-2) var(--s-3) 0;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: var(--s-2);
    list-style: none;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-2);
    padding: var(--s-4) var(--s-3);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .empty p {
    margin: 0;
  }
</style>

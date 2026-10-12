<!--
  StudioRow: a Control layer row's Studio mode parts (lazy studio chunk), rendered by LayerRow in
  three places (`part`):
    tallies   PGM / PVW: the Program tally (on air) and the Preview tally (shown in Preview), each a
              TallyDot whose label carries the state
    acts      Cue in / Cue out (back to "as Program" when that is what Program shows)
    notes     "In preview" while edits wait in Preview; "Program changed since cue" (amber) when
              automation changed the layer on Program after it was cued

  Props:
    ctl: ControlCtl
    screenId: string
    layer: LiveLayerView        the layer on Program
    part: "tallies" | "acts" | "notes"
-->
<script lang="ts">
  import Button from "$lib/components/Button.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { previews } from "$lib/stores/preview.svelte";
  import type { LiveLayerView } from "$lib/types";
  import type { ControlCtl } from "../control.svelte";
  import { cue as cueLayer, inPreview } from "./ops";

  let {
    ctl,
    screenId,
    layer,
    part,
  }: { ctl: ControlCtl; screenId: string; layer: LiveLayerView; part: "tallies" | "acts" | "notes" } = $props();

  const pv = $derived(previews.views.get(screenId));
  const cue = $derived(pv?.cues.find((c) => c.layerId === layer.id));
  const shown = $derived(inPreview(pv, layer.id, layer.onAir));
</script>

{#if part === "tallies"}
  <span class="tallies">
    <span class="tal" title="Program">
      <TallyDot state={layer.onAir ? "program" : "idle"} label={layer.onAir ? "On air" : "Off air"} />PGM
    </span>
    <span class="tal" title="Preview">
      <TallyDot state={shown ? "preview" : "idle"} label={shown ? "In Preview" : "Not in Preview"} />PVW
    </span>
  </span>
{:else if part === "acts"}
  <Button
    size="sm"
    variant={shown ? "secondary" : "primary"}
    disabled={!ctl.studio.on}
    onclick={() => cueLayer(screenId, layer.id)}
    aria-label="{shown ? 'Cue out' : 'Cue in'} {layer.name}"
    title={shown ? "Take it out of Preview" : "Put it in Preview"}
  >
    {shown ? "Cue out" : "Cue in"}
  </Button>
{:else}
  {#if cue?.staged}<p class="note">In preview</p>{/if}
  {#if cue?.programChanged}<p class="note changed">Program changed since cue</p>{/if}
{/if}

<style>
  .tallies {
    display: inline-flex;
    flex: none;
    flex-direction: column;
    gap: 2px;
  }
  .tal {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    color: var(--text-muted);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: var(--tracking-label);
    line-height: 1;
  }
  .note {
    margin: 0;
    /* Lines up with the layer name: chevron, gap, tallies (~32px), gap. */
    padding: 0 var(--s-2) var(--s-1) calc(var(--control-h-sm) + var(--s-2) + 32px + var(--s-2));
    color: var(--accent-text);
    font-size: var(--fs-xs);
  }
  .changed {
    color: var(--warning);
  }
</style>

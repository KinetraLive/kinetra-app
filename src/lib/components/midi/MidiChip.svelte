<!--
  MidiChip: "MIDI" next to a layer's or cue's hotkey badge in Control when a MIDI control is
  assigned; the source ("Note 36 · Ch 1 · APC40") in its tooltip and for screen readers. Nothing
  when none is. Rides LayerBody's chunk (Control's rows render it through `lazy.body`).

  Props:
    k: string   the layer key "<screen>/<layer>" or "flow/<id>"
-->
<script lang="ts">
  import { midiChips } from "./eager.svelte";

  let { k }: { k: string } = $props();
  const text = $derived(midiChips.map.get(k)?.join(", "));
</script>

{#if text}<span class="chip" title={text}>MIDI<span class="visually-hidden">: {text}</span></span>{/if}

<style>
  .chip {
    flex: none;
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text-muted);
    font-size: var(--fs-xs);
    line-height: 18px;
  }
</style>

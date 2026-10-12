<!--
  SmartGuides: what a drag shows besides the alignment lines (part of the lazy overlays chunk):
  the distance from the dragged box to its nearest neighbour or the screen edge on each side, and
  equal-spacing markers (a band over each gap that equals the dragged box's gap). Labels are mono
  screen px. Only transforms change per frame; nothing is read from layout.

  Props:
    smart: Smart | null   from smart.ts, this frame
    scale: number         CSS px per screen px
-->
<script lang="ts">
  import type { Gap, Smart } from "./smart";

  let { smart, scale }: { smart: Smart | null; scale: number } = $props();

  /** The segment's transform: a 1 px element scaled along its axis. */
  function line(g: Gap): string {
    const n = Math.max(0, (g.b - g.a) * scale);
    return g.axis === "x"
      ? `translate(${g.a * scale}px, ${Math.round(g.at * scale)}px) scaleX(${n})`
      : `translate(${Math.round(g.at * scale)}px, ${g.a * scale}px) scaleY(${n})`;
  }
  function band(g: Gap): string {
    const n = Math.max(0, (g.b - g.a) * scale);
    return g.axis === "x"
      ? `translate(${g.a * scale}px, ${g.at * scale - 6}px) scaleX(${n})`
      : `translate(${g.at * scale - 6}px, ${g.a * scale}px) scaleY(${n})`;
  }
  function label(g: Gap): string {
    const mid = ((g.a + g.b) / 2) * scale;
    return g.axis === "x"
      ? `translate(${mid}px, ${g.at * scale}px) translate(-50%, -125%)`
      : `translate(${g.at * scale}px, ${mid}px) translate(6px, -50%)`;
  }
</script>

{#if smart}
  <div class="smart" aria-hidden="true">
    {#each smart.spacing as g, i (i)}
      <div class="band {g.axis}" data-smart="spacing" style:transform={band(g)}></div>
    {/each}
    {#each smart.measures as g, i (i)}
      <div class="seg {g.axis}" data-smart="measure" style:transform={line(g)}></div>
      <div class="label mono" style:transform={label(g)}>{Math.round(g.b - g.a)}</div>
    {/each}
    {#each smart.spacing as g, i (i)}
      {#if !smart.measures.includes(g)}
        <div class="label mono eq" style:transform={label(g)}>{Math.round(g.b - g.a)}</div>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .smart {
    position: absolute;
    inset: 0;
    z-index: 3;
    pointer-events: none;
  }
  .seg,
  .band,
  .label {
    position: absolute;
    top: 0;
    left: 0;
    transform-origin: 0 0;
  }
  .seg {
    background: var(--accent);
  }
  .seg.x {
    width: 1px;
    height: 1px;
  }
  .seg.y {
    width: 1px;
    height: 1px;
  }
  .band {
    background: var(--accent-soft);
  }
  .band.x {
    width: 1px;
    height: 12px;
  }
  .band.y {
    width: 12px;
    height: 1px;
  }
  .label {
    padding: 0 3px;
    border-radius: 3px;
    background: var(--accent);
    color: var(--accent-ink);
    font-size: var(--fs-xs);
    line-height: 15px;
    white-space: nowrap;
  }
  .label.eq {
    background: var(--surface-1);
    color: var(--accent-text);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
</style>

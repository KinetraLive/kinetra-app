<!--
  AnchorPad: a layer's position anchor in the Inspector (and for each layer of a multi-selection).
  A 3×3 grid (Top left … Bottom right) sets both axes; "Center horizontally" / "Center vertically"
  set one; "Respect safe area" anchors inside title safe. Pressing what is already set lets that
  axis go (free). The core keeps anchored layers placed (`anchor.ts` mirrors it for the canvas).

  The grid is one tab stop (roving tabindex); arrow keys move within it, Home / End to the first
  / last cell, Enter / Space choose. A cell is pressed when both axes match; with one axis
  anchored, its row or column is marked.

  Props:
    anchor: LayerAnchor | null | "mixed"   the layer's anchor (null = free; "mixed" = layers differ)
    onanchor: (patch: Partial<LayerAnchor>) => void   applied on top of each layer's own anchor
-->
<script lang="ts">
  import Switch from "$lib/components/Switch.svelte";
  import type { LayerAnchor } from "$lib/types";
  import type { AnchorH, AnchorV } from "./anchor";
  import { ANCHOR_H, ANCHOR_V, anchorLabel } from "./anchor-pad";

  let {
    anchor,
    onanchor,
  }: {
    anchor: LayerAnchor | null | "mixed";
    onanchor: (patch: Partial<LayerAnchor>) => void;
  } = $props();

  const a = $derived(anchor === "mixed" ? null : anchor);
  const h = $derived(a?.h ?? null);
  const v = $derived(a?.v ?? null);
  const safe = $derived(a?.safe ?? false);

  const cells = ANCHOR_V.flatMap((cv) => ANCHOR_H.map((ch) => ({ h: ch, v: cv, label: anchorLabel(ch, cv) })));

  /** The cell holding the tab stop: the anchored one, else the centre. */
  let focusIdx = $state<number | null>(null);
  const pressedIdx = $derived(h && v ? ANCHOR_V.indexOf(v) * 3 + ANCHOR_H.indexOf(h) : -1);
  const tabIdx = $derived(focusIdx ?? (pressedIdx >= 0 ? pressedIdx : 4));

  let grid: HTMLDivElement | undefined = $state();

  function pick(ch: AnchorH, cv: AnchorV) {
    // Clicking the anchored point again lets the layer go free.
    if (ch === h && cv === v) onanchor({ h: null, v: null });
    else onanchor({ h: ch, v: cv });
  }

  function onkeydown(e: KeyboardEvent & { currentTarget: HTMLButtonElement }) {
    const buttons = [...(grid?.querySelectorAll<HTMLButtonElement>("button") ?? [])];
    const i = Math.max(0, buttons.indexOf(e.currentTarget));
    let next = i;
    const row = Math.floor(i / 3);
    const col = i % 3;
    switch (e.key) {
      case "ArrowLeft":
        next = row * 3 + Math.max(0, col - 1);
        break;
      case "ArrowRight":
        next = row * 3 + Math.min(2, col + 1);
        break;
      case "ArrowUp":
        next = Math.max(0, row - 1) * 3 + col;
        break;
      case "ArrowDown":
        next = Math.min(2, row + 1) * 3 + col;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = 8;
        break;
      default:
        return;
    }
    e.preventDefault();
    focusIdx = next;
    buttons[next]?.focus();
  }
</script>

<div class="anchor">
  <div
    class="pad"
    role="group"
    aria-label="Anchor"
    bind:this={grid}
    onfocusout={(e) => {
      if (!grid?.contains(e.relatedTarget as Node | null)) focusIdx = null;
    }}
  >
    {#each cells as c, i (c.label)}
      {@const pressed = c.h === h && c.v === v}
      {@const axis = !pressed && ((h !== null && v === null && c.h === h) || (v !== null && h === null && c.v === v))}
      <button
        type="button"
        class="cell vf"
        class:pressed
        class:axis
        aria-label={c.label}
        aria-pressed={pressed}
        data-tip={c.label}
        tabindex={i === tabIdx ? 0 : -1}
        {onkeydown}
        onclick={() => {
          focusIdx = i;
          pick(c.h, c.v);
        }}
      >
        <span class="dot" aria-hidden="true"></span>
      </button>
    {/each}
  </div>
  <div class="side">
    <button
      type="button"
      class="toggle vf"
      aria-pressed={h === "center"}
      onclick={() => onanchor({ h: h === "center" ? null : "center" })}
    >
      Center horizontally
    </button>
    <button
      type="button"
      class="toggle vf"
      aria-pressed={v === "middle"}
      onclick={() => onanchor({ v: v === "middle" ? null : "middle" })}
    >
      Center vertically
    </button>
    <Switch label="Respect safe area" checked={safe} onchange={(on) => onanchor({ safe: on })} />
  </div>
</div>

<style>
  .anchor {
    display: flex;
    align-items: flex-start;
    gap: var(--s-3);
  }
  .pad {
    display: grid;
    grid-template-columns: repeat(3, var(--control-h-sm));
    grid-template-rows: repeat(3, var(--control-h-sm));
    gap: 2px;
    flex: none;
    padding: 2px;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .cell {
    --vf-radius: var(--r-control);
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    color: var(--text-muted);
  }
  .cell:hover {
    background: var(--surface-3);
    color: var(--text);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    opacity: 0.55;
  }
  .cell.axis {
    color: var(--accent-text);
  }
  .cell.axis .dot {
    opacity: 1;
    box-sizing: border-box;
    border: 1.5px solid currentColor;
    background: transparent;
  }
  .cell.pressed,
  .cell.pressed:hover {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .cell.pressed .dot {
    width: 8px;
    height: 8px;
    opacity: 1;
  }
  /* Tooltip on hover / keyboard focus (like IconButton's). */
  .cell[data-tip]:hover::before,
  .cell[data-tip]:focus-visible::before {
    content: attr(data-tip);
    position: absolute;
    bottom: calc(100% + 4px);
    left: 50%;
    z-index: 50;
    width: max-content;
    padding: var(--s-1) var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-3);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
    font-size: var(--fs-sm);
    line-height: 1.35;
    pointer-events: none;
    transform: translateX(-50%);
  }

  .side {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    min-width: 0;
  }
  .toggle {
    --vf-radius: var(--r-control);
    position: relative;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-sm);
    text-align: left;
    white-space: nowrap;
  }
  .toggle:hover {
    background: var(--surface-3);
  }
  .toggle[aria-pressed="true"],
  .toggle[aria-pressed="true"]:hover {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
</style>

<!--
  SafeAreasPanel: add / edit / remove the screen's custom safe areas (named rectangles in screen
  px, for LED wall bezels and cut-offs). A small panel over the canvas, a lazy chunk opened from
  the toolbar or the palette; Esc or Close closes it. Each change calls `onchange` with the full
  list (the editor saves it in the screen file). Editor-only: nothing reaches the output.

  Props:
    areas: SafeArea[]
    screen: { width, height }
    onchange: (next: SafeArea[]) => void
    onclose: () => void
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import Plus from "lucide-svelte/icons/plus";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import X from "lucide-svelte/icons/x";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import type { Rect, SafeArea } from "$lib/types";
  import { SAFE_AREA_NAME_MAX, SCREEN_SAFE_AREAS_MAX, clampRect, newLayerId } from "./shared";

  let {
    areas,
    screen,
    onchange,
    onclose,
  }: {
    areas: readonly SafeArea[];
    screen: { width: number; height: number };
    onchange: (next: SafeArea[]) => void;
    onclose: () => void;
  } = $props();

  const uid = $props.id();
  let root: HTMLElement | undefined = $state();
  const size = $derived({ w: screen.width, h: screen.height });

  function patch(id: string, p: Partial<SafeArea>) {
    onchange(areas.map((a) => (a.id === id ? { ...a, ...p } : a)));
  }

  function setRect(a: SafeArea, p: Partial<Rect>) {
    patch(a.id, { rect: clampRect({ ...a.rect, ...p }, size) });
  }

  async function add() {
    if (areas.length >= SCREEN_SAFE_AREAS_MAX) return;
    const w = Math.round(screen.width / 2);
    const h = Math.round(screen.height / 2);
    const a: SafeArea = {
      id: newLayerId(),
      name: `Safe area ${areas.length + 1}`,
      rect: { x: Math.round((screen.width - w) / 2), y: Math.round((screen.height - h) / 2), w, h },
    };
    onchange([...areas, a]);
    await tick();
    root?.querySelector<HTMLInputElement>(`[data-area="${a.id}"] input[type="text"]`)?.select();
  }

  onMount(() => {
    root?.querySelector<HTMLElement>("input, button")?.focus();
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section
  class="panel"
  aria-labelledby="{uid}-title"
  bind:this={root}
  onkeydown={(e) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      onclose();
    }
  }}
>
  <header class="head">
    <h2 class="section-label" id="{uid}-title">Custom safe areas</h2>
    <span class="count">{areas.length} / {SCREEN_SAFE_AREAS_MAX}</span>
    <span class="spacer"></span>
    <IconButton size="sm" label="Close" tooltip="left" onclick={onclose}><X size={14} /></IconButton>
  </header>

  {#if areas.length === 0}
    <p class="muted">Keep graphics clear of bezels and cut-offs.</p>
  {/if}
  <ul class="areas">
    {#each areas as a (a.id)}
      <li class="area" data-area={a.id}>
        <span class="name vf vf-within">
          <input
            type="text"
            aria-label="Safe area name"
            autocomplete="off"
            maxlength={SAFE_AREA_NAME_MAX}
            value={a.name}
            aria-invalid={a.name.trim() === "" ? "true" : undefined}
            oninput={(e) => patch(a.id, { name: e.currentTarget.value })}
          />
        </span>
        <IconButton variant="danger" size="sm" label="Remove {a.name}" tooltip="left" onclick={() => onchange(areas.filter((x) => x.id !== a.id))}>
          <Trash2 size={14} />
        </IconButton>
        <span class="rect">
          <label class="coord"><span>X</span><NumberInput value={a.rect.x} digits={4} aria-label="{a.name} X" onchange={(n) => setRect(a, { x: n })} /></label>
          <label class="coord"><span>Y</span><NumberInput value={a.rect.y} digits={4} aria-label="{a.name} Y" onchange={(n) => setRect(a, { y: n })} /></label>
          <label class="coord"><span>W</span><NumberInput value={a.rect.w} min={1} digits={4} aria-label="{a.name} width" onchange={(n) => setRect(a, { w: n })} /></label>
          <label class="coord"><span>H</span><NumberInput value={a.rect.h} min={1} digits={4} aria-label="{a.name} height" onchange={(n) => setRect(a, { h: n })} /></label>
        </span>
      </li>
    {/each}
  </ul>
  <div class="foot">
    <Button size="sm" disabled={areas.length >= SCREEN_SAFE_AREAS_MAX} onclick={() => void add()}>
      {#snippet icon()}<Plus size={14} />{/snippet}
      Add safe area
    </Button>
  </div>
</section>

<style>
  .panel {
    position: absolute;
    top: var(--s-2);
    right: var(--s-2);
    z-index: 5;
    display: flex;
    flex-direction: column;
    width: min(26rem, calc(100% - 2 * var(--s-2)));
    max-height: calc(100% - 2 * var(--s-2));
    border: 1px solid var(--line);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    overflow: auto;
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    padding: var(--s-2) var(--s-2) var(--s-2) var(--s-3);
    border-bottom: 1px solid var(--line);
  }
  .count {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .spacer {
    flex: 1;
  }
  .muted {
    padding: var(--s-3);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .areas {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .area {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: var(--s-2);
    align-items: center;
    padding: var(--s-2) var(--s-3);
    border-bottom: 1px solid var(--line);
  }
  .name {
    display: flex;
    min-width: 0;
    border-radius: var(--r-control);
  }
  .name input {
    width: 100%;
    min-width: 0;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    font-size: var(--fs-sm);
  }
  .name input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .rect {
    grid-column: 1 / -1;
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: var(--s-1);
  }
  .coord {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .foot {
    padding: var(--s-2) var(--s-3);
  }
</style>

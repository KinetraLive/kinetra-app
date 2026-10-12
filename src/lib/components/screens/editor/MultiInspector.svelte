<!--
  MultiInspector: the right column while two or more layers are selected (a lazy chunk). The
  selection's bounds in mono: X / Y are editable and move the whole group; W / H are read-only
  (a group isn't resized as one); "Content sizing" for every selected layer ("Mixed" when they differ). Align
  (to the bounds) and distribute (3 or more), the anchor (AnchorPad: applied to each layer, each
  placed on its own; nothing pressed when they differ), and the selected layers' names. "Fit box to
  graphic" sets each layer whose template has a design to its design box (fitBox) with Scale to
  fit, in one edit.

  Props:
    layers: ScreenLayer[]                 the selected layers, doc order
    onmove: (dx: number, dy: number) => void
    onsizing: (sizing: ContentSizing) => void   content sizing for every selected layer
    templates?: ReadonlyMap<string, TemplateDoc>   the loaded templates (for their design boxes)
    screen?: { width, height }
    onfit?: (changes: { id: string; rect: Rect }[]) => void   Fit box to graphic
    onalign: (kind: AlignKind) => void
    ondistribute: (axis: "x" | "y") => void
    onanchor: (patch: Partial<LayerAnchor>) => void   anchors each selected layer
-->
<script lang="ts">
  import Scan from "lucide-svelte/icons/scan";
  import Button from "$lib/components/Button.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Select from "$lib/components/Select.svelte";
  import type { ContentSizing, LayerAnchor, Rect, ScreenLayer, TemplateDoc } from "$lib/types";
  import AlignBar from "./AlignBar.svelte";
  import AnchorPad from "./AnchorPad.svelte";
  import { commonAnchor } from "./anchor-pad";
  import type { AlignKind } from "./align";
  import { boundsOf, fitBox, sameRect } from "./shared";

  let {
    layers,
    onmove,
    onsizing,
    onalign,
    ondistribute,
    onanchor,
    templates = new Map(),
    screen = { width: 1920, height: 1080 },
    onfit = () => {},
  }: {
    layers: readonly ScreenLayer[];
    onmove: (dx: number, dy: number) => void;
    onsizing: (sizing: ContentSizing) => void;
    onalign: (kind: AlignKind) => void;
    ondistribute: (axis: "x" | "y") => void;
    onanchor: (patch: Partial<LayerAnchor>) => void;
    templates?: ReadonlyMap<string, TemplateDoc>;
    screen?: { width: number; height: number };
    onfit?: (changes: { id: string; rect: Rect }[]) => void;
  } = $props();

  /** Fit box to graphic: every selected layer whose template has a design box, not fitted yet. */
  const fits = $derived.by(() => {
    const out: { id: string; rect: Rect }[] = [];
    let any = false;
    for (const l of layers) {
      if (l.kind === "media") continue;
      const design = templates.get(l.templateId)?.design;
      if (!design) continue;
      any = true;
      const rect = fitBox(l.rect, design, screen);
      if (l.sizing !== "fit" || !sameRect(rect, l.rect)) out.push({ id: l.id, rect });
    }
    return any ? out : null;
  });

  const uid = $props.id();
  const b = $derived(boundsOf(layers.map((l) => l.rect)) ?? { x: 0, y: 0, w: 0, h: 0 });

  /** The selection's sizing (absent = reflow), or "" when the layers differ. */
  const sizing = $derived.by<ContentSizing | "">(() => {
    // Media layers have no content sizing (their fit places the picture).
    const all = new Set(layers.flatMap((l) => (l.kind !== "media" ? [l.sizing ?? "reflow"] : [])));
    return all.size === 1 ? [...all][0] : "";
  });
  const sizingOptions = $derived<{ value: ContentSizing | ""; label: string; disabled?: boolean }[]>([
    ...(sizing === "" ? [{ value: "" as const, label: "Mixed", disabled: true }] : []),
    { value: "fit", label: "Scale to fit" },
    { value: "reflow", label: "Reflow" },
  ]);
</script>

<div class="multi">
  <section class="block" aria-labelledby="{uid}-sel">
    <h2 class="section-label" id="{uid}-sel">Selection</h2>
    <p class="count">{layers.length} layers</p>
    <ul class="names">
      {#each [...layers].reverse() as l (l.id)}
        <li>{l.name}</li>
      {/each}
    </ul>
  </section>

  <section class="block" aria-labelledby="{uid}-box">
    <h2 class="section-label" id="{uid}-box">Bounds</h2>
    <div class="rect">
      <label class="coord">
        <span>X</span>
        <NumberInput value={b.x} digits={5} suffix="px" aria-label="Selection X" onchange={(n) => onmove(n - b.x, 0)} />
      </label>
      <label class="coord">
        <span>Y</span>
        <NumberInput value={b.y} digits={5} suffix="px" aria-label="Selection Y" onchange={(n) => onmove(0, n - b.y)} />
      </label>
      <label class="coord">
        <span>W</span>
        <NumberInput value={b.w} digits={5} suffix="px" aria-label="Selection width" disabled />
      </label>
      <label class="coord">
        <span>H</span>
        <NumberInput value={b.h} digits={5} suffix="px" aria-label="Selection height" disabled />
      </label>
    </div>
    {#if layers.some((l) => l.kind !== "media")}
    <div class="sizing">
      <label for="{uid}-sizing">Content sizing</label>
      <Select
        id="{uid}-sizing"
        class="grow"
        value={sizing}
        options={sizingOptions}
        onchange={(v) => {
          if (v !== "" && v !== sizing) onsizing(v);
        }}
      />
    </div>
    {/if}
    {#if fits}
      <Button size="sm" variant="ghost" class="fit-box" disabled={fits.length === 0} onclick={() => fits && onfit(fits)}>
        {#snippet icon()}<Scan size={12} />{/snippet}
        Fit box to graphic
      </Button>
    {/if}
    <AlignBar count={layers.length} {onalign} {ondistribute} />
    <AnchorPad anchor={commonAnchor(layers)} {onanchor} />
  </section>
</div>

<style>
  .multi {
    display: flex;
    flex-direction: column;
  }
  .block {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding: var(--s-4);
    border-bottom: 1px solid var(--line);
  }
  .count {
    font-size: var(--fs-sm);
  }
  .names {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    list-style: none;
  }
  .names li {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rect {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--s-2) var(--s-3);
  }
  .coord {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .coord > span {
    width: 1ch;
  }
  .sizing {
    display: grid;
    grid-template-columns: minmax(5rem, auto) minmax(0, 1fr);
    gap: var(--s-3);
    align-items: center;
  }
  .sizing :global(.grow) {
    width: 100%;
  }
  .block > :global(.fit-box) {
    align-self: flex-start;
  }
</style>

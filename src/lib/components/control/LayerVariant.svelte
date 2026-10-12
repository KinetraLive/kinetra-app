<!--
  LayerVariant: the Variant select of an expanded Control layer row whose template has several
  variants (part of LayerBody's chunk), and the transition an on-air switch uses (Crossfade / Out and in,
  remembered for the app session). Choosing a variant switches at once off air; on air it
  follows Live edit (onTake: "Changes on next take" under the row's head). "Browse styles…" opens
  the style gallery for this layer (its own lazy chunk): Apply to this layer only / to project.

  Props:
    screenId: string
    layer: LiveLayerView
    template: TemplateSummary     the layer's template (its variants load by id + updatedAt)
-->
<script lang="ts">
  import Select from "$lib/components/Select.svelte";
  import { GalleryLauncher } from "$lib/components/templates/gallery/launcher.svelte";
  import { TRANSITIONS, variantCache, variantOptions } from "$lib/components/screens/variants.svelte";
  import type { LiveLayerView, TemplateSummary } from "$lib/types";
  import Segmented from "./Segmented.svelte";
  import { session, switchVariant } from "./variants.svelte";

  let { screenId, layer, template }: { screenId: string; layer: LiveLayerView; template: TemplateSummary } = $props();

  const loaded = $derived(variantCache.template(template.id, template.updatedAt));
  const current = $derived(layer.variant || loaded?.defaultVariant || "");
  const options = $derived(loaded ? variantOptions(loaded) : [{ value: current, label: "Loading variants" }]);
  const gallery = new GalleryLauncher();
</script>

<div class="variant">
  <span class="section-label">Variant</span>
  <Select
    size="sm"
    aria-label="Variant of {layer.name}"
    disabled={!loaded}
    value={current}
    {options}
    onchange={(v) => void switchVariant(screenId, layer.id, v)}
  />
  <Segmented
    label="Transition on air"
    options={TRANSITIONS}
    value={session.transition}
    onchange={(t) => (session.transition = t)}
  />
  <button
    type="button"
    class="styles-link vf"
    onclick={gallery.show}
  >
    Browse styles…
  </button>
</div>
{#if gallery.component}
  <gallery.component
    open={gallery.open}
    layer={{ screenId, layerId: layer.id, name: layer.name, templateId: template.id }}
    onclose={gallery.close}
  />
{/if}

<style>
  .styles-link {
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    color: var(--accent-text);
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .styles-link:hover {
    background: var(--surface-2);
  }
  .variant {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
  }
</style>

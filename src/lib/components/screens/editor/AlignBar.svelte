<!--
  AlignBar: align left / centre / right / top / middle / bottom and distribute horizontally /
  vertically, for the Inspector (one layer: aligns to the screen) and the multi-selection
  inspector (aligns to the selection's bounds; distribute needs 3 or more). Icon buttons with the
  palette shortcuts in their tooltips.

  Props:
    count: number                                selected layers
    onalign: (kind: AlignKind) => void
    ondistribute: (axis: "x" | "y") => void
-->
<script lang="ts">
  import AlignCenterHorizontal from "lucide-svelte/icons/align-center-horizontal";
  import AlignCenterVertical from "lucide-svelte/icons/align-center-vertical";
  import AlignEndHorizontal from "lucide-svelte/icons/align-end-horizontal";
  import AlignEndVertical from "lucide-svelte/icons/align-end-vertical";
  import AlignHorizontalDistributeCenter from "lucide-svelte/icons/align-horizontal-distribute-center";
  import AlignStartHorizontal from "lucide-svelte/icons/align-start-horizontal";
  import AlignStartVertical from "lucide-svelte/icons/align-start-vertical";
  import AlignVerticalDistributeCenter from "lucide-svelte/icons/align-vertical-distribute-center";
  import IconButton from "$lib/components/IconButton.svelte";
  import type { AlignKind } from "./align";
  import { ALIGN_TITLES, keyHint } from "./commands";

  let {
    count,
    onalign,
    ondistribute,
  }: { count: number; onalign: (kind: AlignKind) => void; ondistribute: (axis: "x" | "y") => void } = $props();

  const ICONS = {
    left: AlignStartVertical,
    center: AlignCenterVertical,
    right: AlignEndVertical,
    top: AlignStartHorizontal,
    middle: AlignCenterHorizontal,
    bottom: AlignEndHorizontal,
  };
  const KINDS: AlignKind[] = ["left", "center", "right", "top", "middle", "bottom"];
  const to = $derived(count === 1 ? " to the screen" : "");
  // Tooltips show the keys after the user's bindings.
  const distX = $derived(keyHint("distributeX", "Distribute horizontally"));
  const distY = $derived(keyHint("distributeY", "Distribute vertically"));
</script>

<div class="bar" role="group" aria-label="Align and distribute">
  {#each KINDS as k (k)}
    {@const Icon = ICONS[k]}
    {@const hint = keyHint(k, `${ALIGN_TITLES[k]}${to}`)}
    <IconButton size="sm" label={hint.label} aria-keyshortcuts={hint.aria} disabled={count === 0} onclick={() => onalign(k)}>
      <Icon size={14} />
    </IconButton>
  {/each}
  <span class="sep" aria-hidden="true"></span>
  <IconButton
    size="sm"
    label={distX.label}
    aria-keyshortcuts={distX.aria}
    disabled={count < 3}
    onclick={() => ondistribute("x")}
  >
    <AlignHorizontalDistributeCenter size={14} />
  </IconButton>
  <IconButton
    size="sm"
    label={distY.label}
    aria-keyshortcuts={distY.aria}
    disabled={count < 3}
    onclick={() => ondistribute("y")}
  >
    <AlignVerticalDistributeCenter size={14} />
  </IconButton>
</div>

<style>
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px;
  }
  .sep {
    width: 1px;
    height: var(--s-4);
    margin: 0 var(--s-1);
    background: var(--line);
  }
</style>

<!--
  Toolbar: the slim bar above the layout editor's canvas (a lazy chunk). Toggles for snapping,
  rulers and guides, the grid (+ its size: 8 / 16 / 32 px or a custom value), action safe, title
  safe, the centre cross and custom safe areas (+ the panel to edit them), and the crop overlay
  (none / 9:16 / 4:5 / 1:1 / 4:3). Every toggle is an icon button with a tooltip naming its
  shortcut and aria-pressed. View options are written to the project's ui-state by the editor.

  Props:
    view: ScreenEditorView
    onview: (patch: Partial<ScreenEditorView>) => void
    guideCount: number
    onclearguides: () => void
    safePanelOpen: boolean
    onsafepanel: () => void     open / close the custom safe areas panel
-->
<script lang="ts">
  import Crosshair from "lucide-svelte/icons/crosshair";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import Frame from "lucide-svelte/icons/frame";
  import Grid3x3 from "lucide-svelte/icons/grid-3x3";
  import Magnet from "lucide-svelte/icons/magnet";
  import Ruler from "lucide-svelte/icons/ruler";
  import Scan from "lucide-svelte/icons/scan";
  import SquareDashed from "lucide-svelte/icons/square-dashed";
  import SquarePen from "lucide-svelte/icons/square-pen";
  import IconButton from "$lib/components/IconButton.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Select from "$lib/components/Select.svelte";
  import type { CropOverlay, ScreenEditorView } from "$lib/types";
  import { SCREEN_GRID_SIZE_RANGE } from "./shared";
  import { keyHint } from "./commands";

  // Tooltips show the keys after the user's bindings (Preferences > Keyboard shortcuts).
  const snapKey = $derived(keyHint("snap", "Snapping"));
  const guidesKey = $derived(keyHint("guides", "Rulers and guides"));
  const gridKey = $derived(keyHint("grid", "Grid"));
  const safeKey = $derived(keyHint("safe", "Action safe 93%"));
  const cropKey = $derived(keyHint("crop", "Crop overlay"));
  const cropLabel = $derived(cropKey.aria ? cropKey.label.replace(/\)$/, " cycles)") : cropKey.label);

  let {
    view,
    onview,
    guideCount,
    onclearguides,
    safePanelOpen,
    onsafepanel,
  }: {
    view: ScreenEditorView;
    onview: (patch: Partial<ScreenEditorView>) => void;
    guideCount: number;
    onclearguides: () => void;
    safePanelOpen: boolean;
    onsafepanel: () => void;
  } = $props();

  const PRESETS = ["8", "16", "32"];
  type GridChoice = "8" | "16" | "32" | "custom";
  let customGrid = $state(false);
  const gridChoice = $derived<GridChoice>(
    !customGrid && PRESETS.includes(String(view.gridSize)) ? (String(view.gridSize) as GridChoice) : "custom",
  );
  const GRID_OPTIONS: { value: GridChoice; label: string }[] = [
    { value: "8", label: "8 px" },
    { value: "16", label: "16 px" },
    { value: "32", label: "32 px" },
    { value: "custom", label: "Custom" },
  ];
  const CROP_OPTIONS: { value: CropOverlay; label: string }[] = [
    { value: "none", label: "No crop" },
    { value: "9:16", label: "9:16" },
    { value: "4:5", label: "4:5" },
    { value: "1:1", label: "1:1" },
    { value: "4:3", label: "4:3" },
  ];

  function pickGrid(v: GridChoice) {
    customGrid = v === "custom";
    if (v !== "custom") onview({ gridSize: Number(v), grid: true });
  }
</script>

<div class="toolbar" role="toolbar" aria-label="Layout tools">
  <IconButton
    size="sm"
    label={snapKey.label}
    aria-keyshortcuts={snapKey.aria}
    pressed={view.snap}
    onclick={() => onview({ snap: !view.snap })}
  >
    <Magnet size={14} />
  </IconButton>

  <span class="sep" aria-hidden="true"></span>
  <IconButton
    size="sm"
    label={guidesKey.label}
    aria-keyshortcuts={guidesKey.aria}
    pressed={view.guides}
    onclick={() => onview({ guides: !view.guides })}
  >
    <Ruler size={14} />
  </IconButton>
  <IconButton size="sm" label="Clear guides" disabled={guideCount === 0} onclick={onclearguides}>
    <Trash2 size={14} />
  </IconButton>

  <span class="sep" aria-hidden="true"></span>
  <IconButton size="sm" label={gridKey.label} aria-keyshortcuts={gridKey.aria} pressed={view.grid} onclick={() => onview({ grid: !view.grid })}>
    <Grid3x3 size={14} />
  </IconButton>
  <Select size="sm" aria-label="Grid size" value={gridChoice} options={GRID_OPTIONS} onchange={pickGrid} />
  {#if gridChoice === "custom"}
    <NumberInput
      value={view.gridSize}
      min={SCREEN_GRID_SIZE_RANGE.min}
      max={SCREEN_GRID_SIZE_RANGE.max}
      digits={3}
      suffix="px"
      aria-label="Custom grid size"
      onchange={(n) => onview({ gridSize: n, grid: true })}
    />
  {/if}

  <span class="sep" aria-hidden="true"></span>
  <IconButton
    size="sm"
    label={safeKey.label}
    aria-keyshortcuts={safeKey.aria}
    pressed={view.actionSafe}
    onclick={() => onview({ actionSafe: !view.actionSafe })}
  >
    <Scan size={14} />
  </IconButton>
  <IconButton size="sm" label="Title safe 90%" pressed={view.titleSafe} onclick={() => onview({ titleSafe: !view.titleSafe })}>
    <SquareDashed size={14} />
  </IconButton>
  <IconButton
    size="sm"
    label="Center cross"
    pressed={view.centerCross}
    onclick={() => onview({ centerCross: !view.centerCross })}
  >
    <Crosshair size={14} />
  </IconButton>
  <IconButton
    size="sm"
    label="Custom safe areas"
    pressed={view.customSafe}
    onclick={() => onview({ customSafe: !view.customSafe })}
  >
    <Frame size={14} />
  </IconButton>
  <IconButton size="sm" label="Edit custom safe areas" pressed={safePanelOpen} pressedFill={false} onclick={onsafepanel}>
    <SquarePen size={14} />
  </IconButton>

  <span class="sep" aria-hidden="true"></span>
  <Select
    size="sm"
    aria-label={cropLabel}
    aria-keyshortcuts={cropKey.aria}
    value={view.crop}
    options={CROP_OPTIONS}
    onchange={(crop: CropOverlay) => onview({ crop })}
  />
</div>

<style>
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-1);
    min-height: var(--row-h);
    padding: var(--s-1) var(--s-3);
  }
  .sep {
    width: 1px;
    height: var(--s-4);
    margin: 0 var(--s-1);
    background: var(--line);
  }
</style>

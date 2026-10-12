<!--
  LayerPick: the screen and layer pickers of a node aimed at a layer (shared by the inspector
  forms). A new screen picks its top layer; layer-dependent parts (fields, a timer key) follow
  (flows/pickers setScreen / setLayer). A target that no longer exists stays listed as missing.

  Props:
    ed: FlowEditorCtl
    value: Layered                 the node's config (not mutated)
    onchange: (next) => void       a changed copy
    disabled?: boolean
-->
<script lang="ts" generics="T extends Layered">
  import Select from "$lib/components/Select.svelte";
  import { layersOf, screenOptions, setLayer, setScreen, withCurrent, type Layered } from "../../pickers";
  import type { FlowEditorCtl } from "../editor.svelte";
  import Field from "./Field.svelte";

  let { ed, value, onchange, disabled = false }: { ed: FlowEditorCtl; value: T; onchange: (next: T) => void; disabled?: boolean } =
    $props();

  function change(f: (x: T) => void) {
    const next = structuredClone($state.snapshot(value)) as T;
    f(next);
    onchange(next);
  }
</script>

<Field label="Screen">
  <Select
    size="sm"
    class="grow"
    aria-label="Screen"
    {disabled}
    value={value.screenId}
    options={withCurrent(screenOptions(ed.lookups), value.screenId, "Choose a screen")}
    onchange={(v) => change((x) => setScreen(ed.lookups, x, v))}
  />
</Field>
<Field label="Layer">
  <Select
    size="sm"
    class="grow"
    aria-label="Layer"
    {disabled}
    value={value.layerId}
    options={withCurrent(layersOf(ed.lookups, value.screenId), value.layerId, "Choose a layer")}
    onchange={(v) => change((x) => setLayer(ed.lookups, x, v))}
  />
</Field>

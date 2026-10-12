<!-- Test stand-in for sheet/SheetEditor.svelte (vi.mock): records its props, and buttons fire the
     callbacks the real grid would. -->
<script lang="ts">
  import type { SheetEditorProps } from "$lib/components/sheet/types";

  let props: SheetEditorProps = $props();
  $effect.pre(() => {
    (globalThis as { __sheet?: SheetEditorProps }).__sheet = props;
  });
</script>

<div role="grid" aria-label={props.title} data-rows={props.rows.length} data-limit={props.limits.rows}>
  <button type="button" onclick={() => props.onchange([...props.rows, { name: "New" }], props.columns, [...props.rows.map((_, i) => i), -1])}>
    Stub add row
  </button>
  <button type="button" onclick={() => props.onchange(props.rows.slice(1), props.columns, props.rows.slice(1).map((_, i) => i + 1))}>
    Stub delete first
  </button>
  <input aria-label="Stub cell" onkeydown={(e) => e.key === "Escape" && e.preventDefault()} />
  <button type="button" onclick={() => props.onclose?.()}>Stub close</button>
</div>

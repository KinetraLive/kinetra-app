<!--
  /flows/[id]: the flow editor. Remounts per id, so switching flows (e.g. after Duplicate) flushes
  the old one's pending draft and loads the new one fresh.
-->
<script lang="ts">
  import { page } from "$app/state";
  import FlowEditor from "$lib/components/flows/graph/FlowEditor.svelte";

  const id = $derived(page.params.id ?? "");
</script>

{#key id}
  <FlowEditor {id} />
{/key}

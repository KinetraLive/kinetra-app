<!--
  /screens/[id]: the screen layout editor. Remounts per id, so switching screens (e.g. after
  Duplicate) flushes the old one's pending save and loads the new one fresh.
-->
<script lang="ts">
  import { page } from "$app/state";
  import ScreenEditor from "$lib/components/screens/editor/ScreenEditor.svelte";

  const id = $derived(page.params.id ?? "");
</script>

{#key id}
  <ScreenEditor {id} />
{/key}

<!--
  /templates/[id]: the template editor. Remounts per id, so switching templates (e.g. after
  Duplicate) flushes the old one's pending save and loads the new one fresh.
-->
<script lang="ts">
  import { page } from "$app/state";
  import TemplateEditor from "$lib/components/templates/editor/TemplateEditor.svelte";

  const id = $derived(page.params.id ?? "");
</script>

{#key id}
  <TemplateEditor {id} />
{/key}

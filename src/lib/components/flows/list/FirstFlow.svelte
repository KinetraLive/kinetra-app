<!--
  FirstFlow: the /flows empty state (a project without flows; its own small chunk). What flows do,
  "Create your first flow" (a flow with a Cue trigger, opened in the editor), "Start from a template"
  (the flow template gallery) and "Add recipe".

  Props:
    oncreate: () => void
    ontemplate: () => void
    onrecipe: () => void
-->
<script lang="ts">
  import Plus from "lucide-svelte/icons/plus";
  import Workflow from "lucide-svelte/icons/workflow";
  import Button from "$lib/components/Button.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";

  let { oncreate, ontemplate, onrecipe }: { oncreate: () => void; ontemplate: () => void; onrecipe: () => void } =
    $props();
</script>

<EmptyState
  title="Let flows run the show"
  description="Take graphics on schedule, on time or from one cue."
>
  {#snippet icon()}<Workflow size={28} />{/snippet}
  {#snippet actions()}
    <Button variant="primary" onclick={oncreate}>
      {#snippet icon()}<Plus size={14} />{/snippet}
      Create your first flow
    </Button>
    <span class="more">
      <Button onclick={ontemplate}>Start from a template</Button>
      <Button onclick={onrecipe}>Add recipe</Button>
    </span>
  {/snippet}
</EmptyState>

<style>
  /* The two secondary actions stay together (on their own row under the primary when narrow). */
  .more {
    display: inline-flex;
    gap: var(--s-2);
  }
</style>

<!--
  FlowsHidden: what /flows and /flows/[id] show in Simple mode (a lazy chunk of the shell's SimpleGate):
  the "Available in Advanced mode" hint with its one-click switch. No props.
-->
<!-- Runes mode: without runes Svelte compiles legacy mode, whose flag module becomes a chunk the picker loads. -->
<svelte:options runes />

<script lang="ts">
  import AdvancedHint from "$lib/components/AdvancedHint.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
</script>

<svelte:head><title>Flows · Kinetra</title></svelte:head>

<div class="simple">
  <EmptyState title="Flows">
    <AdvancedHint />
  </EmptyState>
</div>

<style>
  .simple {
    display: grid;
    place-items: center;
    min-height: 60vh;
    padding: var(--s-6) var(--s-4);
  }
</style>

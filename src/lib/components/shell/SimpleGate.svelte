<!--
  The routed page, or what Simple mode shows instead: /flows and /flows/[id] don't mount and show
  the "Available in Advanced mode" hint (FlowsHidden, a lazy chunk). The project's flows keep
  running. In the shell rather than a flows layout: a third layout level grows the route table
  every window loads (the picker's budget).

  Props:
    path: string         the current URL path
    children?: Snippet   the routed page
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { isAdvanced } from "$lib/stores/mode.svelte";

  let { path, children }: { path: string; children?: Snippet } = $props();

  const flowsHidden = $derived(!isAdvanced() && /^\/flows(\/|$)/.test(path));
</script>

{#if flowsHidden}
  {#await import("$lib/components/flows/hidden").then((m) => m.loadFlowsHidden()) then { default: FlowsHidden }}
    <FlowsHidden />
  {/await}
{:else}
  {@render children?.()}
{/if}

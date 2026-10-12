<!--
  EdgeView: a wire on the canvas (part of the lazy canvas chunk). Event wires are 2 px solid
  --accent, data wires 1.5 px solid --text-muted (their ports' shapes differ too). Selected: drawn
  in --text. Live debug: an accent copy of the path fades in and out over it (opacity only;
  reduced motion holds it for the flash).

  Props: EdgeProps of @xyflow/svelte (`type` is the wire kind: "event" | "data").
-->
<script lang="ts">
  import { BaseEdge, getBezierPath, type EdgeProps } from "@xyflow/svelte";
  import { useEditor } from "../context";

  let { id, type, selected, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition }: EdgeProps = $props();

  const ed = useEditor();
  const path = $derived(getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition })[0]);
  const flashing = $derived(ed.debug.edges.has(id));
</script>

<BaseEdge {id} {path} class={["wire", type, selected && "selected"]} interactionWidth={16} />
{#if flashing}<path d={path} class="glow" fill="none" />{/if}

<style>
  :global(.svelte-flow__edge-path.wire) {
    stroke-linecap: round;
  }
  :global(.svelte-flow__edge-path.wire.event) {
    stroke: var(--accent);
    stroke-width: 2;
  }
  :global(.svelte-flow__edge-path.wire.data) {
    stroke: var(--text-muted);
    stroke-width: 1.5;
  }
  :global(.svelte-flow__edge-path.wire.selected) {
    stroke: var(--text);
  }
  .glow {
    stroke: var(--accent);
    stroke-width: 5;
    stroke-linecap: round;
    opacity: 0;
    pointer-events: none;
    animation: glow 340ms linear;
  }
  @keyframes glow {
    35% {
      opacity: 0.6;
    }
    100% {
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .glow {
      animation: none;
      opacity: 0.6;
    }
  }
</style>

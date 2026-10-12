<!--
  Api: rendered inside SvelteFlow (it needs xyflow's context) to hand the canvas's viewport
  operations to the editor: zoom, fit, the flow position at the middle of the view and at a client
  point (drops from the palette). Renders nothing.

  Props:
    host: HTMLElement | undefined   the canvas element (its middle is "the middle of the view")
    register: (api) => () => void   called once mounted; the returned function on unmount
-->
<script lang="ts">
  import { useSvelteFlow } from "@xyflow/svelte";
  import { onMount } from "svelte";

  type Pos = { x: number; y: number };
  type CanvasViewApi = {
    toFlow(x: number, y: number): Pos;
    center(): Pos;
    zoomIn(): void;
    zoomOut(): void;
    fit(): void;
  };

  let { host, register }: { host: HTMLElement | undefined; register: (api: CanvasViewApi) => () => void } = $props();

  const flow = useSvelteFlow();
  const dur = () => (matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 160);

  onMount(() =>
    register({
      toFlow: (x, y) => flow.screenToFlowPosition({ x, y }),
      center: () => {
        const r = host?.getBoundingClientRect();
        if (!r) return { x: 0, y: 0 };
        return flow.screenToFlowPosition({ x: r.left + r.width / 2, y: r.top + r.height / 2 });
      },
      zoomIn: () => void flow.zoomIn({ duration: dur() }),
      zoomOut: () => void flow.zoomOut({ duration: dur() }),
      fit: () => void flow.fitView({ padding: 0.2, maxZoom: 1, duration: dur() }),
    }),
  );
</script>

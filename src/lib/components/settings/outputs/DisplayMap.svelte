<!--
  Preferences › Outputs: the display map. Each connected display is a rectangle at its desktop
  position, the whole desktop scaled to fit (at most ~180px tall): name, resolution and refresh
  rate (mono), a "Main window" marker on the display Kinetra's window is on, and the outputs
  assigned to it (name · Fill / Key, with a red dot while showing).

  Props:
    displays: DisplayInfo[]
    outputs: OutputConfig[]
    status: OutputStatus[]
-->
<script lang="ts">
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { displayIdOf, mapLayout, resolution } from "$lib/components/outputs/model";
  import type { DisplayInfo, OutputConfig, OutputStatus } from "$lib/types";

  let { displays, outputs, status }: { displays: readonly DisplayInfo[]; outputs: readonly OutputConfig[]; status: readonly OutputStatus[] } =
    $props();

  const HEIGHT = 180;
  const layout = $derived(mapLayout(displays));
  const statusOf = (id: string) => status.find((s) => s.id === id);
  const on = (d: DisplayInfo) => outputs.filter((o) => displayIdOf(o, statusOf(o.id)) === d.id);
</script>

<div class="wrap">
  <ul class="map" aria-label="Displays" style:width="min(100%, {Math.round(HEIGHT * layout.aspect)}px)" style:aspect-ratio={layout.aspect}>
    {#each layout.boxes as b (b.display.id)}
      {@const d = b.display}
      {@const here = on(d)}
      <li
        class="display"
        class:main={d.hasMainWindow}
        style:left="{b.left}%"
        style:top="{b.top}%"
        style:width="calc({b.width}% - 2px)"
        style:height="calc({b.height}% - 2px)"
        data-display={d.id}
      >
        <span class="name" title={d.name}>{d.name}</span>
        <span class="mono meta">{resolution(d)} · {d.refreshHz} Hz</span>
        {#if d.hasMainWindow}<span class="marker">Main window</span>{/if}
        {#if here.length > 0}
          <span class="outs">
            {#each here as o (o.id)}
              {@const showingNow = statusOf(o.id)?.state === "showing"}
              <span class="out" class:off={!o.enabled}>
                {#if showingNow}<TallyDot state="program" pulse={false} size={6} label="Showing" />{/if}
                {o.name} · {o.mode === "key" ? "Key" : "Fill"}
              </span>
            {/each}
          </span>
        {/if}
      </li>
    {/each}
  </ul>
</div>

<style>
  .wrap {
    display: flex;
    justify-content: center;
    padding: var(--s-4) 0;
    border-top: 1px solid var(--line);
  }
  .map {
    position: relative;
    max-height: 180px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .display {
    position: absolute;
    display: flex;
    flex-direction: column;
    gap: 2px;
    /* A 2px gutter between touching displays. */
    margin: 1px;
    padding: var(--s-1) var(--s-2);
    overflow: hidden;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    font-size: var(--fs-xs);
    line-height: 1.3;
    box-sizing: border-box;
  }
  .display.main {
    background: var(--surface-1);
  }
  .name {
    overflow: hidden;
    color: var(--text);
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta,
  .marker {
    overflow: hidden;
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .outs {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: auto;
  }
  .out {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    overflow: hidden;
    color: var(--accent-text);
    white-space: nowrap;
  }
  .out.off {
    color: var(--text-muted);
  }
</style>

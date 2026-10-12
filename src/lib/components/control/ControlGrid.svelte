<!--
  ControlGrid: the visible screens as live preview tiles in `columns` columns.

  Performance (hard rule): iframes follow LiveThumbs: only while the tile is on screen, the window
  is visible and the server runs, at most CONTROL_LIVE_CAP (6) at once, the selected screen first.
  Hidden screens are not passed in, so they have no tile and no iframe.

  Props:
    screens: LiveScreenView[]   visible screens, in order
    columns: number             1–6
    selectedId: string | null
    status: ServerStatus | null
    pending?: ReadonlySet<string>   screens whose Preview differs from Program (Studio mode)
    onselect: (id: string) => void
    onclear: (id: string) => void
-->
<script lang="ts">
  import { LiveThumbs } from "$lib/components/live/thumbs.svelte";
  import { screenUrl } from "$lib/stores/server.svelte";
  import type { LiveScreenView, ServerStatus } from "$lib/types";
  import { CONTROL_LIVE_CAP } from "./control.svelte";
  import ControlTile from "./ControlTile.svelte";

  let {
    screens,
    columns,
    selectedId,
    status,
    pending = new Set(),
    onselect,
    onclear,
  }: {
    screens: readonly LiveScreenView[];
    columns: number;
    selectedId: string | null;
    status: ServerStatus | null;
    pending?: ReadonlySet<string>;
    onselect: (id: string) => void;
    onclear: (id: string) => void;
  } = $props();

  const running = $derived(status?.state === "running");
  const thumbs = new LiveThumbs({
    order: () => screens.map((s) => s.screen.id),
    running: () => running,
    cap: CONTROL_LIVE_CAP,
    first: () => selectedId,
  });
  $effect(() => () => thumbs.destroy());
</script>

<ul class="grid" style:--cols={columns} aria-label="Screens">
  {#each screens as view (view.screen.id)}
    {@const id = view.screen.id}
    <li>
      <ControlTile
        {view}
        url={screenUrl(status, id)}
        live={thumbs.live.has(id)}
        note={thumbs.note(id)}
        width={thumbs.box(id).w}
        selected={id === selectedId}
        pending={pending.has(id)}
        track={thumbs.track(id)}
        onselect={() => onselect(id)}
        onclear={() => onclear(id)}
      />
    </li>
  {/each}
</ul>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
    gap: var(--s-4);
    margin: 0;
    padding: var(--vf-room);
    list-style: none;
  }
  li {
    min-width: 0;
  }
</style>

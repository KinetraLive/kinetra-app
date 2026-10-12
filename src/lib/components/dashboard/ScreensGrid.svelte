<!--
  Dashboard screens grid: one ScreenCard per screen, auto-filling the full width
  (repeat(auto-fill, minmax(280px, 1fr))).

  Props:
    screens: LiveScreenView[]   the user's screens (the dev test screen is filtered out upstream)
    status: ServerStatus | null
    selectedId?: string | null  the screen to mark as selected (brackets)

  Performance (hard rule): live thumbnails follow LiveThumbs ($lib/components/live/thumbs.svelte.ts):
  an iframe only while the card is visible, the window is visible and the server runs, at most 6.
-->
<script lang="ts">
  import { LiveThumbs } from "$lib/components/live/thumbs.svelte";
  import { lanScreenUrl, screenUrl } from "$lib/stores/server.svelte";
  import type { LiveScreenView, ServerStatus } from "$lib/types";
  import { screenTally } from "./data";
  import ScreenCard from "./ScreenCard.svelte";

  type Props = {
    screens: LiveScreenView[];
    status: ServerStatus | null;
    selectedId?: string | null;
  };
  let { screens, status, selectedId = null }: Props = $props();

  const running = $derived(status?.state === "running");
  const thumbs = new LiveThumbs({
    order: () => screens.map((s) => s.screen.id),
    running: () => running,
  });
  $effect(() => () => thumbs.destroy());
</script>

<ul class="grid">
  {#each screens as view (view.screen.id)}
    {@const id = view.screen.id}
    <li>
      <ScreenCard
        {view}
        url={screenUrl(status, id)}
        lanUrl={lanScreenUrl(status, id)}
        tally={screenTally(view, status)}
        live={thumbs.live.has(id)}
        note={thumbs.note(id)}
        width={thumbs.box(id).w}
        selected={selectedId === id}
        track={thumbs.track(id)}
      />
    </li>
  {/each}
</ul>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: var(--s-5) var(--s-4);
    margin: 0;
    /* Room for the tally line and the focus ring outside each card. */
    padding: var(--vf-room);
    list-style: none;
  }
  li {
    min-width: 0;
  }
</style>

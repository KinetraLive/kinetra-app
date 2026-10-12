<!--
  Screens list grid: one ScreenCard per screen, auto-filling the full width.

  Props:
    screens: ScreenSummary[]                 in display order
    views: ReadonlyMap<string, LiveScreenView>   live state per screen id (tally)
    status: ServerStatus | null
    selectedId: string | null
    busy?: boolean
    onselect(id), onopen(id), onduplicate(s), ondelete(s), onrename(s, name) → Promise<boolean>

  Keyboard: arrows / Home / End move the selection (and focus), Delete asks to delete, Enter on a
  card opens it, F2 renames.

  Performance (hard rule): live thumbnails follow LiveThumbs ($lib/components/live/thumbs.svelte.ts):
  an iframe only while the card is visible, the window is visible and the server runs, at most 6.
-->
<script lang="ts">
  import { tick } from "svelte";
  import { LiveThumbs } from "$lib/components/live/thumbs.svelte";
  import { gridColumns, moveIndex } from "$lib/components/grid/keys";
  import { lanScreenUrl, screenUrl } from "$lib/stores/server.svelte";
  import type { LiveScreenView, ScreenSummary, ServerStatus } from "$lib/types";
  import { tallyOf } from "./format";
  import ScreenCard from "./ScreenCard.svelte";

  type Props = {
    screens: ScreenSummary[];
    views: ReadonlyMap<string, LiveScreenView>;
    status: ServerStatus | null;
    selectedId: string | null;
    busy?: boolean;
    onselect: (id: string) => void;
    onopen: (id: string) => void;
    onduplicate: (s: ScreenSummary) => void;
    ondelete: (s: ScreenSummary) => void;
    onrename: (s: ScreenSummary, name: string) => Promise<boolean>;
  };
  let {
    screens,
    views,
    status,
    selectedId,
    busy = false,
    onselect,
    onopen,
    onduplicate,
    ondelete,
    onrename,
  }: Props = $props();

  const running = $derived(status?.state === "running");

  let list: HTMLUListElement | undefined = $state();
  const thumbs = new LiveThumbs({
    order: () => screens.map((s) => s.id),
    running: () => running,
  });
  $effect(() => () => thumbs.destroy());

  function onkeydown(e: KeyboardEvent) {
    if (screens.length === 0) return;
    const index = Math.max(0, screens.findIndex((s) => s.id === selectedId));
    if (e.key === "Delete") {
      const s = screens.find((x) => x.id === selectedId);
      if (s) {
        e.preventDefault();
        ondelete(s);
      }
      return;
    }
    const next = moveIndex(e.key, index, screens.length, gridColumns(list));
    if (next === null) return;
    e.preventDefault();
    const id = screens[next].id;
    onselect(id);
    void tick().then(() =>
      list?.querySelector<HTMLElement>(`[data-screen="${id}"] [data-pick]`)?.focus(),
    );
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<ul class="grid" aria-label="Screens" bind:this={list} {onkeydown}>
  {#each screens as s (s.id)}
    <li>
      <ScreenCard
        screen={s}
        url={screenUrl(status, s.id)}
        lanUrl={lanScreenUrl(status, s.id)}
        tally={tallyOf(views.get(s.id), status)}
        live={thumbs.live.has(s.id)}
        note={thumbs.note(s.id)}
        box={thumbs.box(s.id)}
        selected={s.id === selectedId}
        {busy}
        track={thumbs.track(s.id)}
        onselect={() => onselect(s.id)}
        onopen={() => onopen(s.id)}
        onduplicate={() => onduplicate(s)}
        ondelete={() => ondelete(s)}
        onrename={(name) => onrename(s, name)}
      />
    </li>
  {/each}
</ul>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: var(--s-4);
    margin: 0;
    /* Room for the tally line and the focus ring outside each card. */
    padding: var(--vf-room);
    list-style: none;
  }
  li {
    min-width: 0;
  }
</style>

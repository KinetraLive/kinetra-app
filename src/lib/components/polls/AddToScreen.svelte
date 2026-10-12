<!--
  AddToScreen: puts the Poll graphic on a screen, every field bound to the poll's source (made when
  the poll has none yet). A screen picker and Add; a toast confirms with "Open screen".

  Props:
    source: () => Promise<string | null>   the poll source's id (created on demand)
-->
<script lang="ts">
  import { goto } from "$app/navigation";
  import { toAppError } from "$lib/api";
  import { addPollToScreen } from "$lib/api/polls";
  import { listScreenDocs } from "$lib/api/screens";
  import Button from "$lib/components/Button.svelte";
  import Select from "$lib/components/Select.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { ScreenSummary } from "$lib/types";

  let { source }: { source: () => Promise<string | null> } = $props();

  let screens = $state.raw<ScreenSummary[] | null>(null);
  let screenId = $state("");
  let busy = $state(false);

  $effect(() => {
    let live = true;
    listScreenDocs().then(
      (list) => {
        if (!live) return;
        screens = list;
        screenId = list[0]?.id ?? "";
      },
      () => live && (screens = []),
    );
    return () => (live = false);
  });

  async function add() {
    if (!screenId || busy) return;
    busy = true;
    try {
      const sourceId = await source();
      if (!sourceId) return;
      const state = await addPollToScreen(sourceId, screenId);
      const id = state.doc.id;
      toasts.push({
        kind: "success",
        message: `Added the poll to ${state.doc.name}`,
        action: { label: "Open screen", run: () => void goto(`/screens/${encodeURIComponent(id)}`) },
      });
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't add the poll to the screen: ${toAppError(e).message}` });
    } finally {
      busy = false;
    }
  }
</script>

{#if screens && screens.length > 0}
  <span class="add">
    <Select
      size="sm"
      aria-label="Screen"
      options={screens.map((s) => ({ value: s.id, label: s.name }))}
      bind:value={screenId}
    />
    <Button size="sm" variant="primary" loading={busy} onclick={add}>Add to screen</Button>
  </span>
{:else if screens}
  <span class="muted">Create a screen to show the poll.</span>
{/if}

<style>
  .add {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
  }
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
</style>

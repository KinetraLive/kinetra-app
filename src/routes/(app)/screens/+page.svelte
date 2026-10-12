<!--
  Screens. Full width: a grid of screen cards (live thumbnail under the dashboard's rules, tally,
  mono URL + copy, resolution, layer count). Click selects, Enter / double-click / Open opens
  /screens/<id>. Duplicate, Rename (in place) and Delete (confirm) per card.

  - listScreenDocs on mount, after each mutation, and when the backend reports a screen file
    changed on disk (`project-files-changed`); no polling. The tally comes from the live store,
    kept current by `live-changed`.
  - Rename and delete are optimistic and roll back on failure.
  - The New screen dialog and the delete confirmation are lazy chunks.
-->
<script module lang="ts">
  /** The last selection, so coming back to the page selects the same screen. */
  let remembered: string | null = null;
</script>

<script lang="ts">
  import { onMount, tick } from "svelte";
  import Plus from "lucide-svelte/icons/plus";
  import { goto } from "$app/navigation";
  import { on, toAppError } from "$lib/api";
  import { live } from "$lib/stores/live.svelte";
  import { deleteScreen, duplicateScreen, listScreenDocs, renameScreen } from "$lib/api/screens";
  import Button from "$lib/components/Button.svelte";
  import ScreenGrid from "$lib/components/screens/list/ScreenGrid.svelte";
  import { sortScreens } from "$lib/components/screens/list/format";
  import { NewScreenLauncher } from "$lib/components/screens/new/launcher.svelte";
  import { server } from "$lib/stores/server.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { SCREEN_NAME_MAX } from "$lib/types";
  import type { ScreenSummary } from "$lib/types";

  type ConfirmType = typeof import("$lib/components/ConfirmDialog.svelte").default;

  $effect(() => server.watch());

  const newScreen = new NewScreenLauncher();

  /** Sorted by name; null until loaded. */
  let list = $state.raw<ScreenSummary[] | null>(null);
  // Live state (tally) from the live store: kept current by `live-changed`.
  $effect(() => live.watch());
  const views = $derived(live.screens);
  let selectedId = $state<string | null>(remembered);
  let busy = $state(false);

  let Confirm = $state.raw<ConfirmType | null>(null);
  let pendingDelete = $state.raw<ScreenSummary | null>(null);
  let deleting = $state(false);

  const names = $derived(list?.map((s) => s.name) ?? []);

  function fail(what: string, e: unknown) {
    toasts.push({ kind: "error", message: `${what}: ${toAppError(e).message}` });
  }

  function select(id: string | null) {
    selectedId = id;
    remembered = id;
  }

  /** Replaces the list; keeps the selection when it still exists, else `prefer`, else the first. */
  function setList(next: ScreenSummary[], prefer?: string | null) {
    list = next;
    const want = prefer !== undefined ? prefer : selectedId;
    select(next.find((s) => s.id === want)?.id ?? next[0]?.id ?? null);
  }

  let gen = 0;
  async function refresh(prefer?: string | null) {
    const g = ++gen;
    try {
      const [docs] = await Promise.all([listScreenDocs(), live.refresh()]);
      if (g !== gen) return;
      setList(sortScreens(docs), prefer);
    } catch (e) {
      if (g !== gen) return;
      list ??= [];
      fail("Couldn't load screens", e);
    }
  }

  onMount(() => {
    void refresh();
    return on("project-files-changed", (p) => {
      if (p.kind === "screen") void refresh();
    });
  });

  /** Puts keyboard focus back on the selected card (after a dialog closes). */
  async function focusSelected() {
    await tick();
    const id = selectedId;
    if (id) document.querySelector<HTMLElement>(`[data-screen="${id}"] [data-pick]`)?.focus();
  }

  function open(id: string) {
    void goto(`/screens/${id}`);
  }

  async function rename(s: ScreenSummary, name: string): Promise<boolean> {
    const before = list;
    if (before) list = sortScreens(before.map((x) => (x.id === s.id ? { ...x, name } : x)));
    try {
      await renameScreen(s.id, name);
      void refresh(s.id);
      return true;
    } catch (e) {
      if (before) list = before;
      fail("Couldn't rename the screen", e);
      return false;
    }
  }

  async function duplicate(s: ScreenSummary) {
    busy = true;
    try {
      const state = await duplicateScreen(s.id, `${s.name} copy`.slice(0, SCREEN_NAME_MAX));
      await refresh(state.doc.id);
    } catch (e) {
      fail("Couldn't duplicate the screen", e);
    } finally {
      busy = false;
    }
  }

  async function askDelete(s: ScreenSummary) {
    try {
      Confirm ??= (await import("$lib/components/ConfirmDialog.svelte")).default;
      pendingDelete = s;
    } catch (e) {
      fail("Couldn't open the dialog", e);
    }
  }

  async function confirmDelete() {
    const s = pendingDelete;
    const before = list;
    if (!s || !before) return;
    deleting = true;
    try {
      await deleteScreen(s.id);
    } catch (e) {
      fail("Couldn't delete the screen", e);
      return;
    } finally {
      deleting = false;
    }
    pendingDelete = null;
    // The neighbour takes its place, right away; then reconcile with the backend.
    const i = before.findIndex((x) => x.id === s.id);
    const rest = before.filter((x) => x.id !== s.id);
    const neighbour = rest[Math.min(Math.max(0, i), rest.length - 1)]?.id ?? null;
    setList(rest, neighbour);
    void focusSelected();
    void refresh(neighbour);
  }

  function cancelDelete() {
    pendingDelete = null;
    void focusSelected();
  }
</script>

<svelte:head><title>Screens · Kinetra</title></svelte:head>

<div class="page">
  <header class="head">
    <h1>Screens</h1>
    <Button variant="primary" onclick={newScreen.show}>
      {#snippet icon()}<Plus size={14} />{/snippet}
      New screen
    </Button>
  </header>

  {#if list && list.length === 0}
    <!-- Only in an empty project: its own small chunk. -->
    <div class="create">
      {#await import("$lib/components/screens/list/CreateFirst.svelte") then { default: CreateFirst }}
        <CreateFirst onclick={newScreen.show} />
      {/await}
    </div>
  {:else if list}
    <ScreenGrid
      screens={list}
      {views}
      status={server.status}
      {selectedId}
      {busy}
      onselect={select}
      onopen={open}
      onduplicate={duplicate}
      ondelete={askDelete}
      onrename={rename}
    />
  {/if}
</div>

{#if newScreen.component}
  {@const NewScreen = newScreen.component}
  <NewScreen open={newScreen.open} onclose={newScreen.close} {names} />
{/if}

{#if Confirm && pendingDelete}
  <Confirm
    title="Delete screen?"
    message={`“${pendingDelete.name}” will be removed from the project and its URL stops working. Backups keep a copy.`}
    confirmLabel="Delete"
    danger
    busy={deleting}
    onconfirm={confirmDelete}
    oncancel={cancelDelete}
  />
{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: var(--s-5);
    min-width: 0;
    padding: var(--s-6);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
    flex-wrap: wrap;
  }
  .create {
    padding: var(--vf-gap);
  }
</style>

<!--
  Sources. Full width: the list of sources on the left (name, kind, mono row count, how many
  screens use it) and the editor for the selected one on the right.

  - listSources on mount, after create/duplicate/delete, and when a source file changes on disk
    (`project-files-changed`, kind "source"); no polling. Saves and steps in the editor patch the
    list entry in place.
  - The New source dialog and the delete confirmation are lazy chunks.
  - The editor is remounted per source (its unmount flushes pending edits). The editors are lazy
    chunks, by kind: the data table editor starts loading with the page, the schedule and API
    editors on the first source of their kind shown.
  - `?source=<id>` (the palette's "Open <source>") selects that source.
  - API sources: `source-changed` patches the entry (its fetch status for the list's dot and
    the API editor's Status); no events at rest.
  - Media: the header's Data / Media switch (`?tab=media`) shows the project's media library (video,
    audio, images; import with progress, codec checks), a lazy chunk.
-->
<script module lang="ts">
  import type { Component } from "svelte";
  import type { ApiStatus, PollSourceView, SourceKindName, SourceSummary } from "$lib/types";

  /** The last selection, so coming back to the page selects the same source. */
  let remembered: string | null = null;

  type EditorProps = {
    id: string;
    usedBy: readonly string[];
    /** API sources: the fetch status (other editors ignore it). */
    api?: ApiStatus | null;
    /** Poll sources: test or linked, connection, state (other editors ignore it). */
    poll?: PollSourceView | null;
    onsaved: (summary: SourceSummary) => void;
    onduplicated: (doc: { id: string }) => void;
    ondeleted: () => void;
  };
  type Editor = Component<EditorProps>;
  const chunks: Partial<Record<SourceKindName, Promise<Editor>>> = {};
  const loaders: Record<SourceKindName, () => Promise<{ default: unknown }>> = {
    text: () => import("$lib/components/sources/SourceEditor.svelte"),
    schedule: () => import("$lib/components/sources/schedule/ScheduleEditor.svelte"),
    api: () => import("$lib/components/sources/api/ApiEditor.svelte"),
    poll: () => import("$lib/components/sources/poll/PollSourceEditor.svelte"),
  };
  /** The editor for a kind (cached; a failed load is retried next time). */
  function loadEditor(kind: SourceKindName): Promise<Editor> {
    const k = kind in loaders ? kind : "text";
    const p = (chunks[k] ??= loaders[k]().then((m) => m.default as Editor));
    p.catch(() => delete chunks[k]);
    return p;
  }
  // The common case starts loading with the route, in parallel with the list.
  void loadEditor("text").catch(() => {});
</script>

<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import Plus from "lucide-svelte/icons/plus";
  import { on, toAppError } from "$lib/api";
  import { listSources } from "$lib/api/sources";
  import Button from "$lib/components/Button.svelte";
  import CreateFirst from "$lib/components/sources/CreateFirst.svelte";
  import SourceList from "$lib/components/sources/SourceList.svelte";
  import { sortSources } from "$lib/components/sources/model";
  import { NewSourceLauncher } from "$lib/components/sources/new/launcher.svelte";
  import Segmented from "$lib/components/control/Segmented.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";


  const newSource = new NewSourceLauncher();

  type Tab = "data" | "media";
  const TABS: { value: Tab; label: string }[] = [
    { value: "data", label: "Data" },
    { value: "media", label: "Media" },
  ];
  const tab = $derived<Tab>(page.url.searchParams.get("tab") === "media" ? "media" : "data");
  function setTab(next: Tab) {
    void goto(next === "media" ? "/sources?tab=media" : "/sources", { replaceState: true, keepFocus: true, noScroll: true });
  }

  /** Sorted by name; null until loaded. */
  let list = $state.raw<SourceSummary[] | null>(null);
  let selectedId = $state<string | null>(remembered);

  const names = $derived(list?.map((s) => s.name) ?? []);
  const selected = $derived(list?.find((s) => s.id === selectedId) ?? null);

  function select(id: string | null) {
    selectedId = id;
    remembered = id;
  }

  let gen = 0;
  /** Reloads the list; keeps the selection when it still exists, else `prefer`, else the first. */
  async function refresh(prefer?: string | null) {
    const g = ++gen;
    try {
      const next = sortSources(await listSources());
      if (g !== gen) return;
      list = next;
      const want = prefer !== undefined ? prefer : selectedId;
      select(next.find((s) => s.id === want)?.id ?? next[0]?.id ?? null);
    } catch (e) {
      if (g !== gen) return;
      list ??= [];
      toasts.push({ kind: "error", message: `Couldn't load sources: ${toAppError(e).message}` });
    }
  }

  onMount(() => {
    void refresh();
    const offFiles = on("project-files-changed", (p) => {
      if (p.kind !== "source") return;
      // The open source deleted on disk: its editor shows a banner; Close refreshes.
      if (p.change === "removed" && p.id === selectedId) return;
      void refresh();
    });
    const offSource = on("source-changed", (s) => {
      if (list?.some((x) => x.id === s.id)) patch(s);
    });
    return () => {
      offSource();
      offFiles();
    };
  });

  // The palette's "Open <source>" lands here as ?source=<id>: select it (kept once the list loads).
  $effect(() => {
    const want = page.url.searchParams.get("source");
    if (!want) return;
    untrack(() => select(want));
    void goto("/sources", { replaceState: true, keepFocus: true, noScroll: true });
  });

  /** A save or step in the editor: patch the entry (name may change the order). */
  function patch(s: SourceSummary) {
    if (!list) return;
    // A save doesn't know the fetch status: the entry keeps the last one.
    list = sortSources(list.map((x) => (x.id === s.id ? { ...s, usedBy: x.usedBy, api: s.api ?? x.api } : x)));
  }

  function created(doc: { id: string }) {
    void refresh(doc.id);
  }

  function afterDelete() {
    if (!list) return;
    const i = list.findIndex((x) => x.id === selectedId);
    const rest = list.filter((x) => x.id !== selectedId);
    const neighbour = rest[Math.min(Math.max(0, i), rest.length - 1)]?.id ?? null;
    list = rest;
    select(neighbour);
    void refresh(neighbour);
  }
</script>

<svelte:head><title>Sources · Kinetra</title></svelte:head>

<div class="page">
  <header class="head">
    <span class="title">
      <h1>Sources</h1>
      <Segmented label="Show" options={TABS} value={tab} onchange={setTab} />
    </span>
    {#if tab === "data"}
      <Button variant="primary" onclick={newSource.show}>
        {#snippet icon()}<Plus size={14} />{/snippet}
        New source
      </Button>
    {/if}
  </header>

  {#if tab === "media"}
    <section class="media" aria-label="Media library">
      {#await import("$lib/components/media/MediaLibrary.svelte") then { default: MediaLibrary }}
        <MediaLibrary />
      {:catch e}
        <p class="load-error">Couldn't open the media library: {toAppError(e).message}</p>
      {/await}
    </section>
  {:else if list && list.length === 0}
    <div class="create"><CreateFirst onclick={newSource.show} /></div>
  {:else if list}
    <div class="split">
      <aside class="left" aria-label="Source list">
        <SourceList sources={list} {selectedId} onselect={select} />
      </aside>
      <section class="right" aria-label="Source editor">
        {#if selected}
          {#key selected.id}
            {#await loadEditor(selected.kind)}
              <!-- Cached after the first source: nothing shows in between. -->
            {:then Editor}
              <Editor
                id={selected.id}
                usedBy={selected.usedBy}
                api={selected.api ?? null}
                poll={selected.poll ?? null}
                onsaved={patch}
                onduplicated={(doc) => void refresh(doc.id)}
                ondeleted={afterDelete}
              />
            {:catch e}
              <p class="load-error">Couldn't open the editor: {toAppError(e).message}</p>
            {/await}
          {/key}
        {/if}
      </section>
    </div>
  {/if}
</div>

{#if newSource.component}
  {@const NewSource = newSource.component}
  <NewSource open={newSource.open} onclose={newSource.close} oncreated={created} {names} />
{/if}

<style>
  .page {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    height: calc(100dvh - var(--topbar-h));
    min-width: 0;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
    flex-wrap: wrap;
    padding: var(--s-5) var(--s-6);
    border-bottom: 1px solid var(--line);
  }
  .title {
    display: inline-flex;
    align-items: center;
    gap: var(--s-4);
  }
  .media {
    min-height: 0;
  }
  .create {
    padding: calc(var(--s-6) + var(--vf-gap));
    align-self: start;
  }
  .split {
    display: grid;
    grid-template-columns: minmax(15rem, 20rem) minmax(0, 1fr);
    min-height: 0;
  }
  .left {
    min-width: 0;
    min-height: 0;
    overflow: auto;
    border-right: 1px solid var(--line);
    background: var(--surface-1);
  }
  .right {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .right > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .load-error {
    padding: var(--s-6);
    color: var(--danger);
  }
  @media (max-width: 760px) {
    .page {
      height: auto;
    }
    .split {
      grid-template-columns: minmax(0, 1fr);
    }
    .left {
      border-right: 0;
      border-bottom: 1px solid var(--line);
    }
  }
</style>

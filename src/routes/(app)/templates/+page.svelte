<!--
  Templates. Full width: the template grid (a listbox of cards) and, for the SELECTED template, one
  preview pane: right of the grid (sticky) when the content is ≥ 1100px wide, under it otherwise.

  - listTemplates once on mount and after each mutation; no polling.
  - Selection loads the template into the backend preview screen (`previewTemplate`, debounced
    150 ms) and puts it on air there (`previewShow`) so the static look shows; leaving the page
    takes it off (`previewHide`). Exactly one preview iframe on this page (PreviewPane).
  - The gallery, the delete confirmation and the empty state are lazy chunks.
  - Keyboard: arrows/Home/End move the selection, Enter/Space select, Delete asks to delete.
    Double-click (or "Open editor") opens /templates/<id>.
-->
<script module lang="ts">
  /** The last selection, so coming back to the page selects the same template. */
  let remembered: string | null = null;
</script>

<script lang="ts">
  import { onMount, tick } from "svelte";
  import Plus from "lucide-svelte/icons/plus";
  import { goto } from "$app/navigation";
  import { toAppError } from "$lib/api";
  import {
    deleteTemplate,
    duplicateTemplate,
    getTemplate,
    listPresets,
    listTemplates,
    previewHide,
    previewShow,
    previewTemplate,
    renameTemplate,
  } from "$lib/api/templates";
  import Button from "$lib/components/Button.svelte";
  import { GalleryLauncher } from "$lib/components/templates/gallery/launcher.svelte";
  import PreviewPane from "$lib/components/templates/list/PreviewPane.svelte";
  import TemplateCard from "$lib/components/templates/list/TemplateCard.svelte";
  import {
    NAME_MAX,
    gridColumns,
    moveIndex,
    originLabel,
    sortTemplates,
  } from "$lib/components/templates/list/format";
  import { screenUrl, server } from "$lib/stores/server.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  // Not from $lib/types: this route needs only the id, not that module's runtime half.
  import { PREVIEW_SCREEN_ID, TEMPLATE_UPDATED } from "$lib/templates-common";
  import type { FieldDef, PresetInfo, TemplateSummary } from "$lib/types";

  type ConfirmType = typeof import("$lib/components/ConfirmDialog.svelte").default;

  const PREVIEW_DEBOUNCE_MS = 150;

  $effect(() => server.watch());
  const previewSrc = $derived(screenUrl(server.status, PREVIEW_SCREEN_ID));

  const gallery = new GalleryLauncher();

  /** Sorted by name; null until loaded. */
  let list = $state.raw<TemplateSummary[] | null>(null);
  let presets = $state.raw<PresetInfo[]>([]);
  let selectedId = $state<string | null>(remembered);
  /** Fields of the selected template, once its full document has loaded. */
  let fields = $state.raw<{ id: string; fields: FieldDef[] } | null>(null);
  let busy = $state(false);
  let now = $state(Date.now());
  let grid: HTMLDivElement | undefined = $state();

  let Confirm = $state.raw<ConfirmType | null>(null);
  let pendingDelete = $state.raw<TemplateSummary | null>(null);
  let deleting = $state(false);

  const selected = $derived(list?.find((t) => t.id === selectedId) ?? null);
  /** The selected id once it exists in the list (a primitive, so list reloads don't re-preview). */
  const previewId = $derived(selected?.id ?? null);
  /** Changes when the selected template takes its preset's latest look (or the undo): preview again. */
  const previewAt = $derived(selected?.preset?.version);

  function fail(what: string, e: unknown) {
    toasts.push({ kind: "error", message: `${what}: ${toAppError(e).message}` });
  }

  /** Reloads the list; keeps the selection when it still exists, else `prefer`, else the first. */
  async function refresh(prefer?: string | null) {
    try {
      const next = sortTemplates(await listTemplates());
      now = Date.now();
      list = next;
      const want = prefer !== undefined ? prefer : selectedId;
      select(next.find((t) => t.id === want)?.id ?? next[0]?.id ?? null);
    } catch (e) {
      list ??= [];
      fail("Couldn't load templates", e);
    }
  }

  function select(id: string | null) {
    selectedId = id;
    remembered = id;
  }

  // A template took its preset's latest look (or the undo), from the side pane or the palette.
  $effect(() => {
    const f = () => void refresh();
    addEventListener(TEMPLATE_UPDATED, f);
    return () => removeEventListener(TEMPLATE_UPDATED, f);
  });

  onMount(() => {
    void refresh();
    listPresets().then(
      (p) => (presets = p),
      () => {
        // Origins fall back to the preset id.
      },
    );
  });

  // Selection → preview screen (debounced) + the full document for the fields list.
  let shown = false;
  let gen = 0;
  $effect(() => {
    const id = previewId;
    void previewAt;
    const g = ++gen;
    if (id === null) {
      if (shown) {
        shown = false;
        previewHide().catch(() => {});
      }
      return;
    }
    const t = setTimeout(async () => {
      getTemplate(id).then(
        (doc) => {
          if (g === gen) fields = { id, fields: doc.fields };
        },
        () => {},
      );
      try {
        await previewTemplate(id);
        if (g !== gen) return;
        await previewShow();
        shown = true;
      } catch (e) {
        if (g === gen) fail("Couldn't preview the template", e);
      }
    }, PREVIEW_DEBOUNCE_MS);
    return () => clearTimeout(t);
  });

  // Leaving the page takes the preview graphic off air.
  $effect(() => () => {
    gen++;
    if (shown) {
      shown = false;
      previewHide().catch(() => {});
    }
  });

  const selectedFields = $derived(fields && fields.id === selectedId ? fields.fields : null);

  function open(id: string) {
    void goto(`/templates/${id}`);
  }

  async function focusSelected() {
    await tick();
    grid?.querySelector<HTMLElement>('[role="option"][aria-selected="true"]')?.focus();
  }

  function onGridKeydown(e: KeyboardEvent) {
    if (!list || list.length === 0) return;
    if (e.key === "Delete") {
      if (selected) {
        e.preventDefault();
        void askDelete(selected);
      }
      return;
    }
    const index = Math.max(0, list.findIndex((t) => t.id === selectedId));
    const next = moveIndex(e.key, index, list.length, gridColumns(grid));
    if (next === null) return;
    e.preventDefault();
    select(list[next].id);
    void focusSelected();
  }

  async function rename(t: TemplateSummary, name: string): Promise<boolean> {
    busy = true;
    try {
      await renameTemplate(t.id, name);
      await refresh(t.id);
      return true;
    } catch (e) {
      fail("Couldn't rename the template", e);
      return false;
    } finally {
      busy = false;
    }
  }

  async function duplicate(t: TemplateSummary) {
    busy = true;
    try {
      const doc = await duplicateTemplate(t.id, `${t.name} copy`.slice(0, NAME_MAX));
      await refresh(doc.id);
    } catch (e) {
      fail("Couldn't duplicate the template", e);
    } finally {
      busy = false;
    }
  }

  async function askDelete(t: TemplateSummary) {
    try {
      Confirm ??= (await import("$lib/components/ConfirmDialog.svelte")).default;
      pendingDelete = t;
    } catch (e) {
      fail("Couldn't open the dialog", e);
    }
  }

  async function confirmDelete() {
    const t = pendingDelete;
    if (!t || !list) return;
    deleting = true;
    try {
      await deleteTemplate(t.id);
      // Select the neighbour that takes its place.
      const i = list.findIndex((x) => x.id === t.id);
      const rest = list.filter((x) => x.id !== t.id);
      const neighbour = rest[Math.min(Math.max(0, i), rest.length - 1)]?.id ?? null;
      pendingDelete = null;
      await refresh(neighbour);
      void focusSelected();
    } catch (e) {
      fail("Couldn't delete the template", e);
    } finally {
      deleting = false;
    }
  }

  function cancelDelete() {
    pendingDelete = null;
    void focusSelected();
  }
</script>

<svelte:head><title>Templates · Kinetra</title></svelte:head>

<div class="frame">
  <div class="layout" class:with-pane={selected !== null}>
    <div class="main">
      <header class="head">
        <h1>Templates</h1>
        <!-- AI (Pro; a lazy chunk): Generate template, or its lock. -->
        {#await import("$lib/components/ai/AiEntry.svelte") then { default: AiEntry }}
          <AiEntry mode={{ kind: "generate" }} />
        {/await}
        <Button variant="primary" onclick={gallery.show}>
          {#snippet icon()}<Plus size={14} />{/snippet}
          New template
        </Button>
      </header>

      {#if list && list.length === 0}
        <!-- Only in an empty project: its own small chunk. -->
        <div class="create">
          {#await import("$lib/components/templates/list/CreateFirst.svelte") then { default: CreateFirst }}
            <CreateFirst onclick={gallery.show} />
          {/await}
        </div>
      {:else if list}
        <div
          class="grid"
          role="listbox"
          aria-label="Templates"
          bind:this={grid}
          onkeydown={onGridKeydown}
          tabindex={selected ? -1 : 0}
        >
          {#each list as t (t.id)}
            <TemplateCard
              template={t}
              origin={originLabel(t.preset, presets)}
              selected={t.id === selectedId}
              {now}
              onselect={() => select(t.id)}
              onopen={() => open(t.id)}
            />
          {/each}
        </div>
      {/if}
    </div>

    {#if selected}
      <aside class="side" aria-label="Selected template">
        <div class="sticky">
          <PreviewPane
            template={selected}
            origin={originLabel(selected.preset, presets)}
            fields={selectedFields}
            src={previewSrc}
            {busy}
            onrename={(name) => rename(selected, name)}
            onduplicate={() => duplicate(selected)}
            ondelete={() => askDelete(selected)}
          />
        </div>
      </aside>
    {/if}
  </div>
</div>

{#if gallery.component}
  {@const Gallery = gallery.component}
  <Gallery open={gallery.open} onclose={gallery.close} />
{/if}

{#if Confirm && pendingDelete}
  <Confirm
    title="Delete template?"
    message={`“${pendingDelete.name}” will be removed from the project. Backups keep a copy.`}
    confirmLabel="Delete"
    danger
    busy={deleting}
    onconfirm={confirmDelete}
    oncancel={cancelDelete}
  />
{/if}

<style>
  /* The size container for the side-pane switch (a grid can't query its own width). */
  .frame {
    container-type: inline-size;
    min-height: calc(100dvh - var(--topbar-h));
    display: grid;
  }
  .layout {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    align-content: start;
  }
  @container (min-width: 1100px) {
    .layout.with-pane {
      grid-template-columns: minmax(0, 1fr) 400px;
      align-content: stretch;
    }
    .side {
      border-top: 0;
      border-left: 1px solid var(--line);
    }
  }

  .main {
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
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: var(--s-3);
    /* Room for the brackets, which sit --vf-gap outside each card. */
    padding: var(--vf-gap);
  }
  .grid:focus-visible {
    outline: none;
  }
  .create {
    padding: var(--vf-gap);
  }

  .side {
    min-width: 0;
    border-top: 1px solid var(--line);
    background: var(--surface-1);
  }
  .sticky {
    position: sticky;
    top: 0;
    padding: var(--s-5);
  }
</style>

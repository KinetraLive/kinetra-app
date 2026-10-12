<!--
  /flows: every flow of the project, grouped by origin (the project's own flows, then one group per
  recipe, by its name). Each row: armed switch, name (opens the editor), trigger summary, notes
  (Running, Not deployed, warnings, read-only), last fired, and Open / Rename / Duplicate /
  Delete…. New flow (also the palette's "New flow", which arrives as ?new) starts Blank (a name,
  then the editor on a flow with one Cue trigger) or From a template (the flow template gallery;
  the first added flow opens in the editor). Without flows: "Create your first flow", "Start from
  a template" and "Add recipe" (the recipe gallery).

  - The list follows `flow-state` (running, last fired) and flow files changed on disk; no polling.
  - Arm and rename are optimistic and roll back on failure; delete asks first.
  - The New flow dialog, the flow template gallery, the delete confirmation, the recipe gallery and
    the trigger summaries are lazy chunks.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import Plus from "lucide-svelte/icons/plus";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { toAppError } from "$lib/api";
  import { createFlow, deleteFlow, duplicateFlow, renameFlow } from "$lib/api/flows";
  import { listRecipeInstances } from "$lib/api/recipes";
  import Button from "$lib/components/Button.svelte";
  import { newNodeId } from "$lib/components/flows/common";
  import FlowRows from "$lib/components/flows/list/FlowRows.svelte";
  import { groupFlows } from "$lib/components/flows/list/groups";
  import { FlowList } from "$lib/components/flows/list/store.svelte";
  import type { Summaries } from "$lib/components/flows/list/summaries.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { uiState } from "$lib/stores/project.svelte";
  import type { FlowInfo } from "$lib/types";

  const flows = new FlowList();
  $effect(() => flows.connect());

  // Trigger summaries (and the screens/sources they name) are a lazy chunk, requested now so it is
  // usually in before the list; rows show their names first.
  let summaries = $state.raw<Summaries | null>(null);
  const summariesReady = import("$lib/components/flows/list/summaries.svelte").then((m) => {
    summaries = new m.Summaries(() => flows.list);
    return summaries;
  });
  summariesReady.catch(() => {});
  $effect(() => summaries?.connect());

  let recipeNames = $state.raw<ReadonlyMap<string, string>>(new Map());

  async function loadRecipes() {
    try {
      recipeNames = new Map(((await listRecipeInstances()) ?? []).map((r) => [r.id, r.name]));
    } catch {
      // Group headings fall back to "Removed recipe".
    }
  }

  onMount(() => {
    void loadRecipes();
  });

  const groups = $derived(groupFlows(flows.list, recipeNames));

  function fail(what: string, e: unknown) {
    toasts.push({ kind: "error", message: `${what}: ${toAppError(e).message}` });
  }

  // ---- New ----
  type NewT = typeof import("$lib/components/flows/list/NewFlowDialog.svelte").default;
  let NewDialog = $state.raw<NewT | null>(null);
  let newOpen = $state(false);

  async function openNew() {
    try {
      NewDialog ??= (await import("$lib/components/flows/list/NewFlowDialog.svelte")).default;
      newOpen = true;
    } catch (e) {
      fail("Couldn't open the dialog", e);
    }
  }

  async function create(name: string): Promise<string> {
    const cue = { id: newNodeId(), type: "trigger.cue", config: { label: null }, position: { x: 0, y: 0 } } as const;
    const f = await createFlow({ name, armed: true, retrigger: "restart", nodes: [cue], edges: [] });
    flows.put(f);
    return f.id;
  }

  // ---- From a template ----
  type TemplatesT = typeof import("$lib/components/flows/templates/FlowTemplateGallery.svelte").default;
  let Templates = $state.raw<TemplatesT | null>(null);
  let templatesOpen = $state(false);

  async function openTemplates() {
    newOpen = false;
    try {
      Templates ??= (await import("$lib/components/flows/templates/FlowTemplateGallery.svelte")).default;
      templatesOpen = true;
    } catch (e) {
      fail("Couldn't open the templates", e);
    }
  }

  function added(list: FlowInfo[]) {
    for (const f of list) flows.put(f);
    if (list[0]) void goto(`/flows/${list[0].id}`);
  }

  // The palette's "New flow" lands here as ?new.
  $effect(() => {
    if (!page.url.searchParams.has("new")) return;
    void openNew();
    void goto("/flows", { replaceState: true, keepFocus: true, noScroll: true });
  });

  // ---- Rename / duplicate / delete ----
  let renaming = $state<string | null>(null);

  async function rename(id: string, name: string): Promise<boolean> {
    const before = flows.get(id);
    if (!before) return false;
    const optimistic = { ...before, name };
    flows.put(optimistic);
    renaming = null;
    try {
      flows.put(await renameFlow(id, name));
      return true;
    } catch (e) {
      if (flows.get(id) === optimistic) flows.put(before);
      fail("Couldn't rename the flow", e);
      return false;
    }
  }

  async function duplicate(id: string) {
    try {
      flows.put(await duplicateFlow(id));
    } catch (e) {
      fail("Couldn't duplicate the flow", e);
    }
  }

  type ConfirmT = typeof import("$lib/components/ConfirmDialog.svelte").default;
  let Confirm = $state.raw<ConfirmT | null>(null);
  let deleting = $state.raw<FlowInfo | null>(null);
  let busy = $state(false);

  async function askDelete(f: FlowInfo) {
    try {
      Confirm ??= (await import("$lib/components/ConfirmDialog.svelte")).default;
      deleting = f;
    } catch (e) {
      fail("Couldn't open the dialog", e);
    }
  }

  async function confirmDelete() {
    const f = deleting;
    if (!f) return;
    busy = true;
    try {
      await deleteFlow(f.id);
    } catch (e) {
      fail("Couldn't delete the flow", e);
      return;
    } finally {
      busy = false;
    }
    deleting = null;
    flows.drop(f.id);
    // The server dropped its hotkey too; mirror that without another write.
    const ui = uiState.value;
    const key = `flow/${f.id}`;
    if (ui && key in ui.control.hotkeys) {
      const { [key]: _gone, ...hotkeys } = ui.control.hotkeys;
      uiState.set({ ...ui, control: { ...ui.control, hotkeys } });
    }
  }

  // ---- Add recipe ----
  type GalleryT = typeof import("$lib/components/recipes/RecipeGallery.svelte").default;
  let Gallery = $state.raw<GalleryT | null>(null);
  let galleryOpen = $state(false);

  async function addRecipe() {
    try {
      const { loadGallery } = await import("$lib/components/recipes/lazy");
      Gallery ??= (await loadGallery()) as unknown as GalleryT;
      await summariesReady; // the gallery offers the project's screens
      galleryOpen = true;
    } catch (e) {
      fail("Couldn't open the recipes", e);
    }
  }
</script>

<svelte:head><title>Flows · Kinetra</title></svelte:head>

<div class="page">
  <header class="head">
    <h1>Flows</h1>
    {#if flows.list.length > 0}
      <span class="actions">
        <Button variant="ghost" onclick={addRecipe}>Add recipe</Button>
        <Button variant="primary" onclick={openNew}>
          {#snippet icon()}<Plus size={14} />{/snippet}
          New flow
        </Button>
      </span>
    {/if}
  </header>

  {#if flows.loaded && flows.list.length === 0}
    <!-- Only in a project without flows: its own small chunk. -->
    {#await import("$lib/components/flows/list/FirstFlow.svelte") then { default: FirstFlow }}
      <FirstFlow oncreate={openNew} ontemplate={openTemplates} onrecipe={addRecipe} />
    {/await}
  {:else}
    {#each groups as g (g.id)}
      <section class="group" data-origin={g.id || "project"} aria-labelledby="group-{g.id || 'project'}">
        <h2 class="section-label" id="group-{g.id || 'project'}">
          {g.name}{#if g.recipe}<span class="kind"> · Recipe</span>{/if}
          <span class="count">{g.flows.length}</span>
        </h2>
        <FlowRows
          flows={g.flows}
          label="{g.name} flows"
          summarize={summaries?.text ?? null}
          {renaming}
          onrename={rename}
          onstartrename={(id) => (renaming = id)}
          oncancelrename={() => (renaming = null)}
          onarm={(id, on) => void flows.setArmed(id, on)}
          onduplicate={(id) => void duplicate(id)}
          ondelete={askDelete}
        />
      </section>
    {/each}
  {/if}
</div>

{#if NewDialog && newOpen}
  <NewDialog
    names={flows.list.map((f) => f.name)}
    oncreate={create}
    ontemplate={openTemplates}
    onclose={() => (newOpen = false)}
  />
{/if}

{#if Templates && templatesOpen}
  <Templates onclose={() => (templatesOpen = false)} onadded={added} />
{/if}

{#if Confirm && deleting}
  <Confirm
    title="Delete “{deleting.name}”?"
    message="The flow and its hotkey are removed. Backups keep a copy."
    confirmLabel="Delete"
    danger
    {busy}
    onconfirm={confirmDelete}
    oncancel={() => (deleting = null)}
  />
{/if}

{#if Gallery}
  <Gallery
    open={galleryOpen}
    onclose={() => (galleryOpen = false)}
    screens={summaries?.screens.list ?? []}
    selectedScreenId={summaries?.screens.list[0]?.screen.id ?? null}
    oninstalled={() => {
      galleryOpen = false;
      void flows.load();
      void summaries?.screens.load();
      void loadRecipes();
    }}
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
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
  }
  .actions {
    display: inline-flex;
    gap: var(--s-2);
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }
  .group h2 {
    display: flex;
    align-items: baseline;
    gap: var(--s-2);
    margin: 0;
    color: var(--text-muted);
  }
  .kind {
    color: var(--text-muted);
  }
  .count {
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0;
  }
</style>

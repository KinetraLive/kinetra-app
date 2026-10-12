<!--
  Dashboard. Full width, two columns: the main column (project header, then the screens grid, or
  for an empty project two large create-actions framed by viewfinder brackets) and a --surface-1 side column flush to the right edge (recent templates
  once the project has content, and the output server/LAN panel). Below 1000px of content width
  the side column moves under the main one.

  Empty vs data: the project is empty when it has no screens of its own (the live store minus the
  hidden test screen) and no templates (`listTemplates()`). Until both lists have loaded, neither
  state renders (no flash of the empty state). With templates but no screens: the screens block
  offers "Create your first screen" (opens the New screen dialog, a lazy chunk) and the side column lists the recent templates.

  "Create a template" (empty state and the Recent templates panel) opens the New template gallery,
  a lazy chunk loaded on first use (GalleryLauncher).
-->
<script lang="ts">
  import FolderOpen from "lucide-svelte/icons/folder-open";
  import LayoutTemplate from "lucide-svelte/icons/layout-template";
  import Monitor from "lucide-svelte/icons/monitor";
  import { toAppError } from "$lib/api";
  import { live } from "$lib/stores/live.svelte";
  import { listPresets, listTemplates } from "$lib/api/templates";
  import IconButton from "$lib/components/IconButton.svelte";
  import CreateAction from "$lib/components/dashboard/CreateAction.svelte";
  import RecentTemplates from "$lib/components/dashboard/RecentTemplates.svelte";
  import ScreensGrid from "$lib/components/dashboard/ScreensGrid.svelte";
  import ServerPanel from "$lib/components/dashboard/ServerPanel.svelte";
  import { isEmptyProject, recentTemplates, userScreens } from "$lib/components/dashboard/data";
  import { NewScreenLauncher } from "$lib/components/screens/new/launcher.svelte";
  import { GalleryLauncher } from "$lib/components/templates/gallery/launcher.svelte";
  import { revealInExplorer } from "$lib/projectFlow";
  import { project } from "$lib/stores/project.svelte";
  import { server } from "$lib/stores/server.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { uiState } from "$lib/stores/uiState.svelte";
  import type { PresetInfo, TemplateSummary } from "$lib/types";

  $effect(() => server.watch());
  const status = $derived(server.status);

  const current = $derived(project.current);
  const dateFmt = new Intl.DateTimeFormat(undefined, { dateStyle: "long" });
  const created = $derived.by(() => {
    const d = current ? new Date(current.createdAt) : null;
    return d && !Number.isNaN(d.getTime()) ? dateFmt.format(d) : null;
  });

  /** All live screens; null until loaded. Reloaded when another project opens. */
  $effect(() => live.watch());
  const all = $derived(live.loaded ? live.list : null);
  /** The project's templates; null until loaded. */
  let templates = $state.raw<TemplateSummary[] | null>(null);
  /** Built-in presets, only for the "From <preset>" origin labels (ids are the fallback). */
  let presets = $state.raw<PresetInfo[]>([]);
  $effect(() => {
    const path = current?.path;
    if (!path) return;
    let alive = true;
    listTemplates().then(
      (list) => {
        if (alive) templates = list;
      },
      (e: unknown) => {
        if (!alive) return;
        templates = [];
        toasts.push({ kind: "error", message: `Couldn't load templates: ${toAppError(e).message}` });
      },
    );
    listPresets().then(
      (list) => {
        if (alive) presets = list;
      },
      () => {},
    );
    return () => {
      alive = false;
    };
  });

  const screens = $derived(all && templates ? userScreens(all) : null);
  const recent = $derived(templates ? recentTemplates(templates) : []);
  const empty = $derived(screens !== null && isEmptyProject(screens, templates ?? []));
  const newScreen = new NewScreenLauncher();
  const gallery = new GalleryLauncher();
  const selectedId = $derived(uiState.value?.control.selectedScreenId ?? null);

  async function reveal() {
    if (!current) return;
    try {
      await revealInExplorer(current.path);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't show the folder: ${toAppError(e).message}` });
    }
  }
</script>

<svelte:head><title>{current?.name ?? "Dashboard"} · Kinetra</title></svelte:head>

{#snippet screenIcon()}<Monitor size={22} />{/snippet}
{#snippet templateIcon()}<LayoutTemplate size={22} />{/snippet}

{#if current}
  <div class="frame">
    <div class="dash">
      <div class="main">
        <header class="head">
          <h1>{current.name}</h1>
          <div class="meta">
            <span class="path mono" title={current.path}>{current.path}</span>
            <IconButton label="Show in Explorer" size="sm" onclick={reveal}>
              <FolderOpen size={14} />
            </IconButton>
          </div>
          {#if created}<p class="created">Created {created}</p>{/if}
        </header>

        {#if screens && empty}
          <section class="block" aria-labelledby="start-h">
            <h2 id="start-h" class="section-label">Get started</h2>
            <div class="create-grid">
              <CreateAction
                onclick={newScreen.show}
                title="Create your first screen"
                description="One browser-source URL for OBS or vMix."
                icon={screenIcon}
              />
              <CreateAction
                onclick={gallery.show}
                title="Create your first template"
                description="Lower thirds and other graphics, ready-made or blank."
                icon={templateIcon}
              />
            </div>
          </section>
        {:else if screens}
          <section class="block" aria-labelledby="screens-h">
            <div class="block-head">
              <h2 id="screens-h" class="section-label">
                Screens <span class="count">{screens.length}</span>
              </h2>
            </div>
            {#if screens.length > 0}
              <ScreensGrid {screens} {status} {selectedId} />
            {:else}
              <div class="create-grid">
                <CreateAction
                  onclick={newScreen.show}
                  title="Create your first screen"
                  description="One browser-source URL for OBS or vMix."
                  icon={screenIcon}
                />
              </div>
            {/if}
          </section>
        {/if}

      </div>

      <aside class="side" aria-label="Project status">
        {#if screens && !empty}
          <div class="side-block"><RecentTemplates templates={recent} {presets} oncreate={gallery.show} /></div>
        {/if}
        <div class="side-block"><ServerPanel {status} /></div>
      </aside>
    </div>
  </div>
{/if}

{#if gallery.component}
  {@const Gallery = gallery.component}
  <Gallery open={gallery.open} onclose={gallery.close} />
{/if}

{#if newScreen.component}
  {@const NewScreen = newScreen.component}
  <NewScreen open={newScreen.open} onclose={newScreen.close} names={screens?.map((s) => s.screen.name)} />
{/if}

<style>
  /* The size container for the two-column switch (a grid can't query its own width). */
  .frame {
    container-type: inline-size;
    /* The content pane's height (the shell grid gives it everything under the top bar), so the
       side column runs to the bottom edge. */
    min-height: calc(100dvh - var(--topbar-h));
    display: grid;
  }
  .dash {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 320px;
    min-height: 100%;
  }
  @container (max-width: 1000px) {
    .dash {
      grid-template-columns: minmax(0, 1fr);
    }
    .side {
      border-top: 1px solid var(--line);
      border-left: 0;
    }
  }

  .main {
    display: flex;
    flex-direction: column;
    gap: var(--s-7);
    min-width: 0;
    padding: var(--s-6);
  }
  .head {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    min-width: 0;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
  }
  .path {
    overflow: hidden;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .created {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }

  .block {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    min-width: 0;
  }
  .block-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
  }
  .count {
    margin-left: var(--s-1);
    color: var(--text);
    font-weight: 400;
    letter-spacing: 0;
  }
  .create-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
    gap: var(--s-5);
    /* Room for the brackets, which sit --vf-gap outside each tile. */
    padding: var(--vf-gap);
  }

  .side {
    display: flex;
    flex-direction: column;
    min-width: 0;
    border-left: 1px solid var(--line);
    background: var(--surface-1);
  }
  .side-block {
    padding: var(--s-5);
  }
  .side-block + .side-block {
    border-top: 1px solid var(--line);
  }
</style>

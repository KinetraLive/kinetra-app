<!--
  Project picker (welcome screen).
  Layout: header (mark + KINETRA wordmark as the h1, one muted line), then two columns:
  actions on the left (New / Open / In the cloud, with the inline New project form under New, the
  "folder isn't empty" confirmation under Open and the account's cloud projects under In the cloud,
  each a lazy chunk), recent projects on the right. One column
  below 760px. The expanded action and keyboard focus draw the viewfinder brackets.
  `/?new=1` (from the top-bar switcher) opens with the New project form expanded.
-->
<script lang="ts">
  import BrandLogo from "$lib/components/brand/BrandLogo.svelte";
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import Cloud from "lucide-svelte/icons/cloud";
  import FolderOpen from "lucide-svelte/icons/folder-open";
  import FolderPlus from "lucide-svelte/icons/folder-plus";
  import { toAppError } from "$lib/api";
  import Card from "$lib/components/Card.svelte";
  import ThemeToggle from "$lib/components/ThemeToggle.svelte";
  import type NewProjectFormComponent from "$lib/components/picker/NewProjectForm.svelte";
  import RecentList from "$lib/components/picker/RecentList.svelte";
  import { describeError, parentDir } from "$lib/components/picker/format";
  import { openFolder, pickFolder } from "$lib/projectFlow";
  import type InitFolderPanelComponent from "$lib/components/picker/InitFolderPanel.svelte";
  import type CloudListComponent from "$lib/components/picker/CloudList.svelte";
  import { project } from "$lib/stores/project.svelte";

  let newOpen = $state(false);
  let form: { focusName(): void } | undefined = $state();

  // The form only shows after "New project", so it stays out of the picker's initial bundle
  // (budget: 50 KB gz). Loading starts as soon as the pointer or focus reaches the New card, so it
  // is normally ready before the click.
  let NewProjectForm = $state.raw<typeof NewProjectFormComponent | null>(null);
  let formLoading: Promise<void> | null = null;
  function loadForm() {
    formLoading ??= import("$lib/components/picker/NewProjectForm.svelte").then((m) => {
      NewProjectForm = m.default;
    });
    return formLoading;
  }
  let newCard: HTMLElement | undefined;

  let picking = $state(false);
  let openError = $state<string | null>(null);
  let confirm = $state<{ path: string; entries: number } | null>(null);
  // Loaded on demand: it only appears after a folder dialog, so it stays out of the initial bundle.
  let InitFolderPanel = $state.raw<typeof InitFolderPanelComponent | null>(null);
  $effect(() => {
    if (confirm && !InitFolderPanel) {
      void import("$lib/components/picker/InitFolderPanel.svelte").then((m) => (InitFolderPanel = m.default));
    }
  });
  // The account's cloud projects: a lazy chunk, loaded like the form (pointer or focus on its card).
  let cloudOpen = $state(false);
  let CloudList = $state.raw<typeof CloudListComponent | null>(null);
  let cloudLoading: Promise<void> | null = null;
  function loadCloud() {
    cloudLoading ??= import("$lib/components/picker/CloudList.svelte").then((m) => {
      CloudList = m.default;
    });
    return cloudLoading;
  }
  /** A downloaded project: open it. */
  async function openDownloaded(path: string) {
    openError = null;
    try {
      const outcome = await openFolder(path);
      if (outcome.status === "opened") toDashboard();
    } catch (e) {
      openError = describeError(toAppError(e));
    }
  }
  let confirmError = $state<string | null>(null);
  let initializing = $state(false);
  let recentsError = $state<string | null>(null);

  /** Default location for a new project: the folder the most recent project lives in. */
  const suggestedLocation = $derived.by(() => {
    const last = project.recents.find((r) => !r.missing);
    return last ? parentDir(last.path) : null;
  });

  let opening = $state(false);

  function toDashboard() {
    void goto("/dashboard");
  }

  function expandNew() {
    void loadForm();
    if (newOpen) form?.focusName();
    else newOpen = true; // the form autofocuses its Name field on mount
  }

  function collapseNew() {
    newOpen = false;
    newCard?.focus();
  }

  async function openProject() {
    if (picking || project.busy) return;
    openError = null;
    confirm = null;
    confirmError = null;
    picking = true;
    let path: string | null;
    try {
      path = await pickFolder("Open a project folder");
    } catch (e) {
      openError = describeError(toAppError(e));
      return;
    } finally {
      picking = false;
    }
    if (!path) return; // cancelled
    opening = true;
    try {
      const outcome = await openFolder(path);
      if (outcome.status === "opened") toDashboard();
      else confirm = { path: outcome.path, entries: outcome.entries };
    } catch (e) {
      openError = describeError(toAppError(e));
    } finally {
      opening = false;
    }
  }

  async function initAnyway() {
    if (!confirm || project.busy) return;
    confirmError = null;
    initializing = true;
    try {
      await project.init(confirm.path);
      toDashboard();
    } catch (e) {
      confirmError = describeError(toAppError(e));
    } finally {
      initializing = false;
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (!(e.ctrlKey || e.metaKey) || e.shiftKey || e.altKey || e.repeat) return;
    const key = e.key.toLowerCase();
    if (key === "n") {
      e.preventDefault();
      expandNew();
    } else if (key === "o") {
      e.preventDefault();
      void openProject();
    }
  }


  onMount(() => {
    // window.location rather than $app/state's page (a chunk the picker would load only for this):
    // SvelteKit updates the history entry before it mounts the page, so both agree here.
    const wantsNew = new URLSearchParams(location.search).has("new");
    if (wantsNew) {
      void loadForm();
      newOpen = true;
    }
    void (async () => {
      try {
        await project.refresh();
        // A webview reload with a project still open: go straight back to it.
        if (project.current && !wantsNew) {
          void goto("/dashboard", { replaceState: true });
          return;
        }
      } catch {
        // No backend (or it failed): stay on the picker.
      }
      try {
        await project.loadRecents();
      } catch (e) {
        recentsError = `Couldn't load recent projects. ${toAppError(e).message}`;
      }
    })();
  });

  $effect(() => project.watchRecents());
</script>

<svelte:window {onkeydown} />

<main class="picker">
  <div class="corner"><ThemeToggle tooltip="left" /></div>

  <div class="frame">
    <header>
      <h1 class="brand"><BrandLogo height={56} badge={false} /></h1>
      <p class="sub">Open a project to start.</p>
    </header>

    <div class="cols">
      <div class="actions">
        <Card
          interactive
          padding="none"
          class="action"
          selected={newOpen}
          aria-expanded={newOpen}
          aria-controls="new-project-form"
          aria-keyshortcuts="Control+N"
          onpointerenter={loadForm}
          onfocus={loadForm}
          onclick={() => (newOpen ? collapseNew() : expandNew())}
          {@attach (el: HTMLElement) => {
            newCard = el;
          }}
        >
          <span class="icon" class:on={newOpen} aria-hidden="true"><FolderPlus size={18} /></span>
          <span class="text">
            <span class="title">New project</span>
            <span class="desc">Start fresh in a new folder.</span>
          </span>
          <kbd class="mono" aria-hidden="true">Ctrl+N</kbd>
        </Card>

        {#if newOpen && NewProjectForm}
          <NewProjectForm
            bind:this={form}
            id="new-project-form"
            {suggestedLocation}
            oncreated={toDashboard}
            oncancel={collapseNew}
          />
        {/if}

        <Card
          interactive
          padding="none"
          class="action"
          aria-keyshortcuts="Control+O"
          aria-busy={opening || undefined}
          aria-describedby={openError ? "open-error" : undefined}
          onclick={openProject}
        >
          <span class="icon" aria-hidden="true"><FolderOpen size={18} /></span>
          <span class="text">
            <span class="title">Open project</span>
            <span class="desc">{opening ? "Opening…" : "Open or set up an existing folder."}</span>
          </span>
          <kbd class="mono" aria-hidden="true">Ctrl+O</kbd>
        </Card>
        {#if openError}<p class="error" id="open-error" role="alert">{openError}</p>{/if}

        {#if confirm && InitFolderPanel}
          <InitFolderPanel
            path={confirm.path}
            entries={confirm.entries}
            busy={project.busy}
            {initializing}
            error={confirmError}
            oninit={initAnyway}
            oncancel={() => (confirm = null)}
          />
        {/if}

        <Card
          interactive
          padding="none"
          class="action"
          selected={cloudOpen}
          aria-expanded={cloudOpen}
          aria-controls="cloud-projects"
          onpointerenter={loadCloud}
          onfocus={loadCloud}
          onclick={() => {
            void loadCloud();
            cloudOpen = !cloudOpen;
          }}
        >
          <span class="icon" class:on={cloudOpen} aria-hidden="true"><Cloud size={18} /></span>
          <span class="text">
            <span class="title">In the cloud</span>
            <span class="desc">Download a project from your cloud backup.</span>
          </span>
        </Card>
        {#if cloudOpen && CloudList}
          <CloudList id="cloud-projects" onopened={openDownloaded} />
        {/if}
      </div>

      <RecentList onopened={toDashboard} loadError={recentsError} />
    </div>
  </div>
</main>

<style>
  .picker {
    position: relative;
    height: 100%;
    overflow-y: auto;
    background: var(--bg);
  }
  .corner {
    position: absolute;
    top: var(--s-2);
    right: var(--s-2);
  }
  .frame {
    display: grid;
    gap: var(--s-6);
    width: 100%;
    max-width: 1160px;
    margin: 0 auto;
    padding: max(var(--s-8), 10vh) var(--s-7) var(--s-8);
  }

  header {
    display: grid;
    gap: var(--s-2);
  }
  .brand {
    display: flex;
    /* The artwork has empty margins around the mark; line the bars up with the cards. */
    margin: 0 0 0 -7px;
    line-height: 0;
  }
  .sub {
    color: var(--text-muted);
  }

  .cols {
    display: grid;
    grid-template-columns: minmax(300px, 400px) minmax(0, 1fr);
    align-items: start;
    gap: var(--s-6);
  }
  @media (max-width: 760px) {
    .frame {
      padding-inline: var(--s-4);
    }
    .cols {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .actions {
    display: grid;
    gap: var(--s-2);
    min-width: 0;
  }
  .actions :global(.card.action) {
    display: grid;
    grid-template-columns: 32px minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--s-3);
    padding: var(--s-3) var(--s-4) var(--s-3) var(--s-3);
  }
  .icon {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: var(--r-control);
    background: var(--surface-3);
    color: var(--accent-text);
  }
  .icon.on {
    background: var(--accent-soft);
  }
  .text {
    display: grid;
    min-width: 0;
  }
  .title {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    font-size: var(--fs-md);
    font-weight: 600;
  }
  .desc {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  kbd {
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text-muted);
    font-size: var(--fs-xs);
    line-height: 18px;
    white-space: nowrap;
  }

  .error {
    color: var(--danger);
    font-size: var(--fs-sm);
  }
</style>

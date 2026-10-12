<!--
  Top-bar project switcher: a menu button showing the open project's name (path in its title).
  The menu shows the current project (marked with a selected tally dot), the other recent
  projects, then New project…, Open… and Close project. Items get viewfinder brackets on focus.

  APG menu-button pattern: Enter/Space/ArrowDown open on the first item, ArrowUp on the last;
  Up/Down/Home/End move, Esc closes and returns focus, Tab or a click outside closes.
  The outside-click listener and the `recents-checked` subscription exist only while open.
  ConfirmDialog (for initializing a non-empty folder) is loaded with import() on demand.
  No props.
-->
<script lang="ts">
  import { tick } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import ChevronDown from "lucide-svelte/icons/chevron-down";
  import FolderOpen from "lucide-svelte/icons/folder-open";
  import FolderPlus from "lucide-svelte/icons/folder-plus";
  import X from "lucide-svelte/icons/x";
  import type ConfirmDialogT from "$lib/components/ConfirmDialog.svelte";
  import Badge from "$lib/components/Badge.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { toAppError } from "$lib/api";
  import { describeEntries, openFolder, pickFolder } from "$lib/projectFlow";
  import { project } from "$lib/stores/project.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { RecentProject } from "$lib/types";
  import { sectionRoot } from "./nav";

  const id = $props.id();
  let open = $state(false);
  let root: HTMLDivElement | undefined = $state();
  let trigger: HTMLButtonElement | undefined = $state();
  let menu: HTMLDivElement | undefined = $state();

  let pending = $state<{ path: string; entries: number } | null>(null);
  let initBusy = $state(false);
  let Dialog = $state.raw<typeof ConfirmDialogT | null>(null);

  const others = $derived(project.recents.filter((r) => r.path !== project.current?.path));

  // Listeners only while the menu is open; the effect's cleanup releases both.
  $effect(() => {
    if (!open) return;
    const unwatch = project.watchRecents();
    const onPointerDown = (e: PointerEvent) => {
      if (root && !root.contains(e.target as Node)) open = false;
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    return () => {
      unwatch();
      document.removeEventListener("pointerdown", onPointerDown, true);
    };
  });

  function fail(prefix: string, e: unknown) {
    toasts.push({ kind: "error", message: `${prefix}: ${toAppError(e).message}` });
  }

  function items(): HTMLElement[] {
    return menu ? [...menu.querySelectorAll<HTMLElement>('[role="menuitem"]')] : [];
  }

  async function show(focus: "first" | "last") {
    open = true;
    project.loadRecents().catch((e: unknown) => fail("Couldn't load recent projects", e));
    await tick();
    const list = items();
    (focus === "first" ? list[0] : list[list.length - 1])?.focus();
  }

  function hide(returnFocus = true) {
    open = false;
    if (returnFocus) trigger?.focus();
  }

  function onTriggerClick() {
    if (open) hide(false);
    else void show("first");
  }

  function onTriggerKeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      void show(e.key === "ArrowDown" ? "first" : "last");
    }
  }

  function onMenuKeydown(e: KeyboardEvent) {
    const list = items();
    const n = list.length;
    const i = list.indexOf(document.activeElement as HTMLElement);
    let next: number;
    switch (e.key) {
      case "ArrowDown":
        next = (i + 1) % n;
        break;
      case "ArrowUp":
        next = i <= 0 ? n - 1 : i - 1;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = n - 1;
        break;
      case "Escape":
        e.preventDefault();
        hide();
        return;
      case "Tab":
        hide(false);
        return;
      default:
        return;
    }
    e.preventDefault();
    list[next]?.focus();
  }

  function onRecentKeydown(e: KeyboardEvent, r: RecentProject) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      void switchTo(r);
    }
  }

  /** After switching projects, stay in the same section (ids under it belonged to the old one). */
  async function landInProject() {
    const target = sectionRoot(page.url.pathname);
    if (target !== page.url.pathname) await goto(target);
  }

  async function switchTo(r: RecentProject) {
    if (r.missing) return;
    hide();
    try {
      await project.open(r.path);
      await landInProject();
    } catch (e) {
      fail(`Couldn't open ${r.name}`, e);
    }
  }

  async function newProject() {
    hide(false);
    try {
      // Navigate first: once we are on the picker the shell's guard is unmounted, so its
      // "no project → /" redirect can't race this navigation and drop `?new=1`.
      await goto("/?new=1");
      await project.close();
    } catch (e) {
      fail("Couldn't close the project", e);
    }
  }

  async function openOther() {
    hide();
    try {
      const path = await pickFolder("Open project");
      if (!path) return;
      const outcome = await openFolder(path);
      if (outcome.status === "opened") {
        await landInProject();
        return;
      }
      const mod = await import("$lib/components/ConfirmDialog.svelte");
      Dialog = mod.default;
      pending = { path: outcome.path, entries: outcome.entries };
    } catch (e) {
      fail("Couldn't open the folder", e);
    }
  }

  // File › Open project… in the native menu (menu/commands.ts OPEN_PROJECT_EVENT) runs "Open…".
  $effect(() => {
    const onOpen = () => void openOther();
    window.addEventListener("kinetra:open-project", onOpen);
    return () => window.removeEventListener("kinetra:open-project", onOpen);
  });

  async function confirmInit() {
    if (!pending) return;
    initBusy = true;
    try {
      await project.init(pending.path);
      pending = null;
      await landInProject();
    } catch (e) {
      fail("Couldn't initialize the folder", e);
    } finally {
      initBusy = false;
    }
  }

  function cancelInit() {
    pending = null;
    trigger?.focus();
  }

  async function closeProject() {
    hide(false);
    try {
      await project.close();
      await goto("/");
    } catch (e) {
      fail("Couldn't close the project", e);
    }
  }
</script>

<div class="switcher" bind:this={root}>
  <button
    bind:this={trigger}
    id="{id}-trigger"
    type="button"
    class="trigger vf"
    aria-haspopup="menu"
    aria-expanded={open}
    aria-controls={open ? `${id}-menu` : undefined}
    title={project.current?.path}
    onclick={onTriggerClick}
    onkeydown={onTriggerKeydown}
  >
    <span class="name">{project.current?.name ?? "No project"}</span>
    <ChevronDown size={14} />
  </button>

  {#if open}
    <div
      bind:this={menu}
      id="{id}-menu"
      class="menu"
      role="menu"
      tabindex="-1"
      aria-labelledby="{id}-trigger"
      onkeydown={onMenuKeydown}
    >
      {#if project.current}
        <div class="group-label section-label" role="presentation">Current</div>
        <div class="current" role="presentation" title={project.current.path}>
          <TallyDot state="selected" />
          <span class="recent">
            <span class="recent-name">{project.current.name}</span>
            <span class="path mono">{project.current.path}</span>
          </span>
        </div>
      {/if}
      {#if others.length > 0}
        <div class="group-label section-label" role="presentation">Recent projects</div>
        {#each others as r (r.path)}
          <div
            role="menuitem"
            tabindex="-1"
            class="item vf"
            aria-disabled={r.missing ? "true" : undefined}
            title={r.path}
            onclick={() => switchTo(r)}
            onkeydown={(e) => onRecentKeydown(e, r)}
          >
            <span class="recent">
              <span class="recent-name">{r.name}</span>
              <span class="path mono">{r.path}</span>
            </span>
            {#if r.missing}<Badge tone="warning">Missing</Badge>{/if}
          </div>
        {/each}
      {:else}
        <div class="none" role="presentation">No other recent projects</div>
      {/if}

      <div class="sep" role="separator"></div>

      <button type="button" role="menuitem" tabindex="-1" class="item vf" onclick={newProject}>
        <FolderPlus size={16} />
        New project…
      </button>
      <button type="button" role="menuitem" tabindex="-1" class="item vf" onclick={openOther}>
        <FolderOpen size={16} />
        Open…
      </button>
      <button type="button" role="menuitem" tabindex="-1" class="item vf" onclick={closeProject}>
        <X size={16} />
        Close project
      </button>
    </div>
  {/if}
</div>

{#if pending && Dialog}
  <Dialog
    title="Initialize this folder?"
    message={`${pending.path} already contains ${describeEntries(pending.entries)}. Project files will be added next to them; nothing is deleted.`}
    confirmLabel="Initialize here"
    busy={initBusy}
    onconfirm={confirmInit}
    oncancel={cancelInit}
  />
{/if}

<style>
  .switcher {
    position: relative;
    min-width: 0;
  }

  .trigger {
    --vf-radius: var(--r-control);
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    max-width: 100%;
    height: var(--control-h);
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    color: var(--text);
    font-size: var(--fs-md);
    font-weight: 500;
  }
  .trigger:hover,
  .trigger[aria-expanded="true"] {
    background: var(--surface-3);
  }
  .trigger :global(svg) {
    flex: none;
    color: var(--text-muted);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Flat panel: surface-3 with a 1px inset line, no shadow. */
  .menu {
    position: absolute;
    z-index: 40;
    top: calc(100% + var(--s-1));
    left: 0;
    width: max-content;
    min-width: 260px;
    max-width: min(420px, calc(100vw - var(--s-4) * 2));
    max-height: calc(100vh - var(--topbar-h) - var(--s-4));
    overflow-x: hidden;
    overflow-y: auto;
    padding: var(--s-1);
    border-radius: var(--r-panel);
    background: var(--surface-3);
    box-shadow: inset 0 0 0 1px var(--line);
    animation: menu-in var(--dur-fast) var(--ease);
  }
  @keyframes menu-in {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }

  .group-label {
    display: flex;
    align-items: center;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
  }
  .none {
    display: flex;
    align-items: center;
    min-height: var(--row-h);
    padding: 0 var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }

  .current,
  .item {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    width: 100%;
    min-height: var(--row-h);
    padding: 2px var(--s-2);
    border-radius: var(--r-control);
    color: var(--text);
    text-align: left;
  }
  .item {
    --vf-radius: var(--r-control);
    cursor: pointer;
  }
  .item :global(svg) {
    flex: none;
    color: var(--text-muted);
  }
  .item:hover {
    background: var(--line);
  }
  .item[aria-disabled="true"] {
    cursor: default;
    color: var(--text-muted);
  }
  .item[aria-disabled="true"]:hover {
    background: transparent;
  }

  .recent {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }
  .recent-name {
    font-weight: 500;
    line-height: 1.3;
  }
  .path {
    overflow: hidden;
    color: var(--text-muted);
    font-size: var(--fs-xs);
    line-height: 1.3;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sep {
    height: 1px;
    margin: var(--s-1) 0;
    background: var(--line);
  }
</style>

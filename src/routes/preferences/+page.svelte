<!--
  The Preferences window (label "prefs", opened by `open_preferences`; outside the app shell: no
  sidebar, no menu, no picker redirect). Two columns inside one scrolling pane, at most 1200px wide
  and centred: a sticky 200px section list on the left (the active section in accent text with a
  6px dot, like the main nav) and the active section on the right. Each section is a lazy chunk
  (settings/sections.ts) loaded when first shown and kept for switching back.

  Deep links: `?section=<id>` on load, and `prefs-section` while open (an id or an old
  `/settings#<anchor>`: "backups" opens Advanced scrolled to Backups and focuses it; "updates"
  opens About and starts an update check). The URL follows the section (replacing it), so a reload
  stays put. Changes apply immediately; the main window follows through the cross-window events.
-->
<script lang="ts">
  import { onMount, tick, type Component } from "svelte";
  import { goto } from "$app/navigation";
  import { on, toAppError } from "$lib/api";
  import {
    DEFAULT_SECTION,
    PREFERENCE_SECTIONS,
    resolveSection,
    type SectionTarget,
  } from "$lib/components/settings/sections";
  import { updateRequest } from "$lib/components/settings/state.svelte";
  import { project } from "$lib/stores/project.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { PreferenceSectionId } from "$lib/types";

  let current = $state<PreferenceSectionId>(DEFAULT_SECTION);
  /** Loaded section bodies, kept for switching back. */
  let bodies = $state.raw<Partial<Record<PreferenceSectionId, Component>>>({});
  let failed = $state<PreferenceSectionId | null>(null);
  let scroller: HTMLElement | undefined = $state();
  let content: HTMLElement | undefined = $state();

  const section = $derived(PREFERENCE_SECTIONS.find((s) => s.id === current) ?? PREFERENCE_SECTIONS[0]);
  const Body = $derived(bodies[current] ?? null);
  const blocked = $derived(!!section.needsProject && !project.current);

  async function load(id: PreferenceSectionId): Promise<boolean> {
    if (bodies[id]) return true;
    const s = PREFERENCE_SECTIONS.find((x) => x.id === id);
    if (!s) return false;
    try {
      const m = await s.load();
      bodies = { ...bodies, [id]: m.default };
      return true;
    } catch (e) {
      failed = id;
      toasts.push({ kind: "error", message: `Couldn't open ${s.title}: ${toAppError(e).message}` });
      return false;
    }
  }

  /** Shows a section; `anchor` scrolls to and focuses a block in it, else the top. */
  async function show(target: SectionTarget, focus: boolean) {
    failed = null;
    current = target.section;
    if (target.check) updateRequest.seq++;
    // The URL follows (a reload stays put). goto, not replaceState: the picker already ships goto.
    if (new URLSearchParams(location.search).get("section") !== target.section) {
      try {
        void goto(`?section=${target.section}`, { replaceState: true, noScroll: true, keepFocus: true }).catch(() => {});
      } catch {
        // The router isn't ready yet (first render): the next switch updates it.
      }
    }
    if (!(await load(target.section)) || current !== target.section) return;
    await tick();
    const el = target.anchor ? document.getElementById(target.anchor) : null;
    if (el) {
      el.scrollIntoView({ block: "start" });
      if (focus) el.focus({ preventScroll: true });
    } else {
      if (scroller) scroller.scrollTop = 0;
      if (focus) content?.focus({ preventScroll: true });
    }
  }

  function pick(id: PreferenceSectionId) {
    if (id !== current) void show({ section: id, anchor: null, check: false }, false);
  }

  function onListKeydown(e: KeyboardEvent & { currentTarget: HTMLElement }) {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
    const buttons = [...e.currentTarget.querySelectorAll<HTMLButtonElement>("button")];
    const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const n = buttons.length;
    const next = e.key === "Home" ? 0 : e.key === "End" ? n - 1 : (i + (e.key === "ArrowDown" ? 1 : -1) + n) % n;
    e.preventDefault();
    buttons[next]?.focus();
    const id = buttons[next]?.dataset.section as PreferenceSectionId | undefined;
    if (id) pick(id);
  }

  onMount(() => {
    const start = resolveSection(new URLSearchParams(location.search).get("section"));
    void show(start, start.anchor !== null);
    // This window has no app shell: restore the open project (and its UI state) here.
    project.refresh().catch((e: unknown) => {
      toasts.push({ kind: "error", message: `Couldn't read the open project: ${toAppError(e).message}` });
    });
    return on("prefs-section", ({ section }) => void show(resolveSection(section), true));
  });
</script>

<svelte:head><title>Preferences · Kinetra</title></svelte:head>

<div class="scroller" bind:this={scroller}>
  <div class="prefs">
    <nav class="side" aria-label="Preferences sections">
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <ul class="list" onkeydown={onListKeydown}>
        {#each PREFERENCE_SECTIONS as s (s.id)}
          <li>
            <button
              type="button"
              class="item vf"
              data-section={s.id}
              aria-current={s.id === current ? "page" : undefined}
              onclick={() => pick(s.id)}
            >
              {#if s.id === current}<span class="dot" aria-hidden="true"></span>{/if}
              {s.title}
            </button>
          </li>
        {/each}
      </ul>
    </nav>

    <main class="content" aria-label={section.title} tabindex="-1" bind:this={content}>
      <h1 class="visually-hidden">Preferences</h1>
      {#if blocked}
        <p class="none">Open a project to change its settings.</p>
      {:else if failed === current}
        <p class="none">This section couldn't be loaded.</p>
      {:else if Body}
        {#key current}
          <div class="body"><Body /></div>
        {/key}
      {/if}
    </main>
  </div>
</div>

<style>
  .scroller {
    height: 100%;
    overflow: auto;
    overscroll-behavior: contain;
    scrollbar-gutter: stable;
    background: var(--bg);
  }
  .prefs {
    display: grid;
    grid-template-columns: 200px minmax(0, 1fr);
    gap: var(--s-6);
    max-width: 1200px;
    min-height: 100%;
    margin-inline: auto;
    padding: 0 var(--s-6);
  }
  .side {
    position: sticky;
    top: 0;
    align-self: start;
    padding: var(--s-6) 0;
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .item {
    --vf-radius: var(--r-control);
    position: relative;
    display: flex;
    align-items: center;
    width: 100%;
    height: var(--row-h);
    padding: 0 var(--s-3);
    border-radius: var(--r-control);
    color: var(--text-muted);
    font-weight: 500;
    text-align: left;
    white-space: nowrap;
  }
  .item:hover {
    background: var(--surface-3);
    color: var(--text);
  }
  /* Active: accent text + a 6px accent dot (never accent text on surface-3). */
  .item[aria-current="page"],
  .item[aria-current="page"]:hover {
    background: transparent;
    color: var(--accent-text);
  }
  .dot {
    position: absolute;
    top: calc(50% - 3px);
    left: -3px;
    width: 6px;
    height: 6px;
    border-radius: var(--r-full);
    background: var(--accent);
    box-shadow: inset 0 0 0 1px var(--focus);
    pointer-events: none;
  }
  .content {
    min-width: 0;
    padding: var(--s-6) 0 var(--s-8);
  }
  .content:focus {
    outline: none;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: var(--s-7);
  }
  .none {
    color: var(--text-muted);
  }
  @media (max-width: 640px) {
    .prefs {
      grid-template-columns: minmax(0, 1fr);
      gap: 0;
    }
    .side {
      position: static;
      padding-bottom: 0;
    }
  }
</style>

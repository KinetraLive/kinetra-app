<!--
  BindMenu: the Inspector's "Bind to source" control for one field. An icon button opens a menu
  of the project's sources; picking one shows its columns (with Back); picking a column binds.
  A schedule source first offers its row (Now / Up next, radio items; Now by default), then its
  columns (every one but `countdown`). With `countdown` (timer fields) only schedules are listed
  and picking one offers "Up next · Countdown" and "Now · Countdown".

  Keyboard: Enter/Space/ArrowDown open the menu on its first item; ArrowUp/ArrowDown/Home/End move;
  Escape (or ArrowLeft in the columns step) goes back / closes and returns focus to the button;
  Tab closes. A press outside closes it (the window listener exists only while open).

  Props:
    field: string                    the field's label (for the accessible names)
    sources: SourceSummary[] | null  null while loading
    onopen: () => void               the menu opened (the editor loads the sources lazily)
    onpick: (binding: SourceBinding) => void
    countdown?: boolean              a timer field: bind a schedule's countdown only
-->
<script lang="ts">
  import { tick } from "svelte";
  import ChevronLeft from "lucide-svelte/icons/chevron-left";
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import Link from "lucide-svelte/icons/link";
  import IconButton from "$lib/components/IconButton.svelte";
  import { ROW_LABEL, bindableColumns } from "$lib/components/fields/binding-label";
  import type { RowSelector, SourceBinding, SourceSummary } from "$lib/types";

  let {
    field,
    sources,
    onopen,
    onpick,
    countdown = false,
  }: {
    field: string;
    sources: readonly SourceSummary[] | null;
    onopen: () => void;
    onpick: (binding: SourceBinding) => void;
    countdown?: boolean;
  } = $props();

  const uid = $props.id();
  let open = $state(false);
  /** The source whose columns are listed (step 2), or null (step 1). */
  let sourceId = $state<string | null>(null);
  let root: HTMLDivElement | undefined = $state();
  let menu: HTMLDivElement | undefined = $state();

  /** A schedule's row (step 2). */
  let row = $state<"now" | "next">("now");
  const ROWS: ReadonlyArray<"now" | "next"> = ["now", "next"];
  const COUNTDOWN_ROWS: ReadonlyArray<"now" | "next"> = ["next", "now"];

  const listed = $derived(countdown ? (sources?.filter((s) => s.kind === "schedule") ?? null) : sources);
  const chosen = $derived(sourceId ? (sources?.find((s) => s.id === sourceId) ?? null) : null);
  const isSchedule = $derived(chosen?.kind === "schedule");
  const columns = $derived(bindableColumns(chosen).filter((c) => c.key !== "countdown"));

  const items = () => [...(menu?.querySelectorAll<HTMLButtonElement>('[role^="menuitem"]') ?? [])];

  async function focusFirst(last = false) {
    await tick();
    const all = items();
    (last ? all.at(-1) : all[0])?.focus();
  }

  function show(last = false) {
    sourceId = null;
    open = true;
    onopen();
    void focusFirst(last);
  }

  function close(refocus: boolean) {
    open = false;
    sourceId = null;
    if (refocus) root?.querySelector<HTMLButtonElement>("button")?.focus();
  }

  function back() {
    const from = sourceId;
    sourceId = null;
    void tick().then(() => {
      const el = menu?.querySelector<HTMLButtonElement>(`[data-source="${from}"]`);
      (el ?? items()[0])?.focus();
    });
  }

  function onButtonKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      show(e.key === "ArrowUp");
    }
  }

  function onMenuKey(e: KeyboardEvent) {
    const all = items();
    const i = all.indexOf(document.activeElement as HTMLButtonElement);
    let next = -1;
    if (e.key === "ArrowDown") next = (i + 1) % all.length;
    else if (e.key === "ArrowUp") next = (i - 1 + all.length) % all.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = all.length - 1;
    else if (e.key === "Escape" || (e.key === "ArrowLeft" && sourceId)) {
      e.preventDefault();
      e.stopPropagation();
      if (sourceId) back();
      else close(true);
      return;
    } else if (e.key === "Tab") {
      close(false);
      return;
    }
    if (next < 0 || all.length === 0) return;
    e.preventDefault();
    all[next]?.focus();
  }

  function pickSource(id: string) {
    sourceId = id;
    row = "now";
    void focusFirst();
  }

  function pickColumn(column: string, at: RowSelector | null = isSchedule ? row : null) {
    const id = sourceId;
    close(true);
    if (!id) return;
    const b: SourceBinding = { sourceId: id, column };
    if (at) b.row = at;
    onpick(b);
  }

  $effect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (root && !root.contains(e.target as Node)) close(false);
    };
    window.addEventListener("pointerdown", onDown, true);
    return () => window.removeEventListener("pointerdown", onDown, true);
  });

  // Sources arrive after the menu opened: focus the first item then.
  $effect(() => {
    if (open && listed && !sourceId && !menu?.contains(document.activeElement)) void focusFirst();
  });
</script>

<div class="bind" bind:this={root}>
  <IconButton
    size="sm"
    label="Bind {field} to source"
    tooltip="left"
    aria-haspopup="menu"
    aria-expanded={open}
    aria-controls={open ? `${uid}-menu` : undefined}
    onclick={() => (open ? close(false) : show())}
    onkeydown={onButtonKey}
  >
    <Link size={14} />
  </IconButton>

  {#if open}
    <!-- svelte-ignore a11y_interactive_supports_focus -->
    <div
      class="menu"
      id="{uid}-menu"
      role="menu"
      aria-label={chosen ? `Columns of ${chosen.name}` : `Bind ${field} to a source`}
      bind:this={menu}
      onkeydown={onMenuKey}
    >
      {#if sources === null}
        <p class="note" role="status">Loading sources…</p>
      {:else if chosen}
        <button type="button" class="item vf back" role="menuitem" tabindex="-1" onclick={back}>
          <span class="icon" aria-hidden="true"><ChevronLeft size={14} /></span>
          <span class="name">{chosen.name}</span>
        </button>
        <div class="sep" role="separator"></div>
        {#if countdown}
          <p class="group section-label" aria-hidden="true">Countdown</p>
          {#each COUNTDOWN_ROWS as r (r)}
            <button type="button" class="item vf" role="menuitem" tabindex="-1" onclick={() => pickColumn("countdown", r)}>
              <span class="name">{ROW_LABEL[r]} · Countdown</span>
            </button>
          {/each}
        {:else}
          {#if isSchedule}
            <div role="group" aria-label="Item">
              <p class="group section-label" aria-hidden="true">Item</p>
              {#each ROWS as r (r)}
                <button
                  type="button"
                  class="item vf"
                  role="menuitemradio"
                  aria-checked={row === r}
                  tabindex="-1"
                  onclick={() => (row = r)}
                >
                  <span class="radio" aria-hidden="true">{row === r ? "●" : "○"}</span>
                  <span class="name">{ROW_LABEL[r]}</span>
                </button>
              {/each}
            </div>
            <div class="sep" role="separator"></div>
          {/if}
          <div role="group" aria-label="Column">
            <p class="group section-label" aria-hidden="true">Column</p>
            {#each columns as col (col.key)}
              <button type="button" class="item vf" role="menuitem" tabindex="-1" onclick={() => pickColumn(col.key)}>
                <span class="name">{col.label}</span>
                <span class="meta mono">{col.key}</span>
              </button>
            {/each}
          </div>
        {/if}
      {:else if listed === null || listed.length === 0}
        <p class="note">
          This project has no {countdown ? "schedules" : "sources"} yet. Create one on the
          <a href="/sources">Sources</a> page.
        </p>
      {:else}
        <p class="group section-label" aria-hidden="true">{countdown ? "Schedule" : "Source"}</p>
        {#each listed as s (s.id)}
          <button
            type="button"
            class="item vf"
            role="menuitem"
            tabindex="-1"
            aria-haspopup="menu"
            data-source={s.id}
            onclick={() => pickSource(s.id)}
          >
            <span class="name">{s.name}</span>
            <span class="meta">{s.rowCount} {s.kind === "schedule" ? "item" : "row"}{s.rowCount === 1 ? "" : "s"}</span>
            <span class="icon" aria-hidden="true"><ChevronRight size={14} /></span>
          </button>
        {/each}
      {/if}
    </div>
  {/if}
</div>

<style>
  .bind {
    position: relative;
    flex: none;
  }
  .menu {
    position: absolute;
    z-index: 20;
    top: calc(100% + var(--s-1));
    right: 0;
    display: flex;
    flex-direction: column;
    width: 16rem;
    max-height: 20rem;
    overflow: auto;
    padding: var(--s-1);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .group {
    padding: var(--s-1) var(--s-2);
  }
  .item {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: var(--row-h);
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    color: var(--text);
    text-align: left;
  }
  .item:hover,
  .item:focus-visible {
    background: var(--surface-2);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .icon {
    display: inline-flex;
    color: var(--text-muted);
  }
  .radio {
    width: 1ch;
    color: var(--accent-text);
    font-size: var(--fs-xs);
  }
  .back .name {
    font-weight: 600;
  }
  .sep {
    height: 1px;
    margin: var(--s-1) 0;
    background: var(--line);
  }
  .note {
    padding: var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .note a {
    color: var(--accent-text);
  }
</style>

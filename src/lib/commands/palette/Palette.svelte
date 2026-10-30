<!--
  Palette: the command palette (Ctrl+K / Ctrl+P / Ctrl+Shift+P from the shell). A lazy chunk: the
  shell imports it on the first open. Lists the registry's commands allowed in the current context
  plus provider items (project items: ../providers, registered on the first open and fed while
  open; asked once per open, again when their data changes while open), searched with the hand-written scorer (search.ts / score.ts), grouped by section: Recent,
  then Live first in Control, Go to, Create, Edit, Project items, Settings. Prefixes: ">" commands,
  "@" screens, "#" templates, "/" settings.

  Keyboard (an ARIA combobox over a grouped listbox): ArrowUp/Down move, Home/End jump to the first
  / last row, Tab / Shift+Tab to the next / previous section, Enter runs, Esc closes. A command with
  `confirm` needs a second Enter (the row turns --danger; any other key or moving cancels). A command
  with `next` opens its follow-up step: a sub-list (searched the same way) or a text input with its
  validation message; Esc, or Backspace on an empty query, steps back.

  A modal <dialog> (the page behind is inert). Running closes it first and gives focus back to where
  it was, so the command acts on what was focused before. Recent is recorded by `runCommand`.

  Props:
    onclose: () => void
    followUp?: string   open straight into this command's follow-up step (its key was pressed);
                        stepping back then closes the palette
-->
<script lang="ts">
  import { backdropClose } from "$lib/components/backdrop";
  import { onMount, tick } from "svelte";
  import Search from "lucide-svelte/icons/search";
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import type { Command, CommandContext, FollowUp, Section } from "../registry.svelte";
  import { MORE_STEP, prepare, searchOptions, searchSections, type PickMatch } from "./search";
  const ALL_SECTIONS: readonly Section[] = ["recent", "live", "goto", "create", "edit", "items", "settings"];
  import { currentContext, getCommand, keysFor, paletteCommands, recentIds, runCommand, runFollowUp, t, watchItems } from "./source";

  let { onclose, followUp }: { onclose: () => void; followUp?: string } = $props();

  const uid = $props.id();
  let dialog: HTMLDialogElement | undefined = $state();
  let input: HTMLInputElement | undefined = $state();
  let list: HTMLElement | undefined = $state();
  let returnFocus: HTMLElement | null = null;
  let closed = false;

  let query = $state("");
  let active = $state(0);
  /** The command id waiting for its second Enter. */
  let armed = $state<string | null>(null);
  let limits = $state<Partial<Record<Section, number>>>({});

  /** The follow-up step in progress (the main query is kept for stepping back). */
  type Step = { command: Command; title: string; next: FollowUp; mainQuery: string; direct: boolean };
  let step = $state.raw<Step | null>(null);

  const ctx: CommandContext = $derived(currentContext());
  // Registered commands + provider items, prepared once (again only when they change).
  const entries = $derived(prepare(paletteCommands(ctx), ctx, t));
  const results = $derived(step ? [] : searchSections(query, entries, ctx, recentIds(), limits));

  const SECTION_LABEL: Record<Section, string> = {
    recent: "Recent",
    live: "Live",
    goto: "Go to",
    create: "Create",
    edit: "Edit",
    items: "Project items",
    settings: "Settings",
  };

  type Row =
    | { kind: "cmd"; key: string; command: Command; title: string; ranges: readonly (readonly [number, number])[] }
    | { kind: "more"; key: string; section: Section; count: number }
    | { kind: "pick"; key: string; id: string; title: string; detail?: string; ranges: readonly (readonly [number, number])[] };
  type Group = { section: Section | null; label: string; rows: { row: Row; i: number }[] };

  const pickOptions = $derived.by(() => {
    const s = step;
    if (!s || s.next.kind !== "pick") return [];
    try {
      return s.next.options(ctx);
    } catch {
      return [];
    }
  });

  /** Groups with flat row indices (the listbox's options in order). */
  const view = $derived.by(() => {
    const groups: Group[] = [];
    const flat: Row[] = [];
    const push = (g: Group, row: Row) => {
      g.rows.push({ row, i: flat.length });
      flat.push(row);
    };
    if (step) {
      if (step.next.kind === "pick") {
        const g: Group = { section: null, label: step.title, rows: [] };
        for (const m of searchOptions(query, pickOptions) as PickMatch[]) {
          push(g, { kind: "pick", key: `p:${m.option.id}`, id: m.option.id, title: m.option.title, detail: m.option.detail, ranges: m.ranges });
        }
        if (g.rows.length > 0) groups.push(g);
      }
      return { groups, flat };
    }
    for (const r of results) {
      const g: Group = { section: r.section, label: t(SECTION_LABEL[r.section]), rows: [] };
      for (const m of r.matches) {
        push(g, { kind: "cmd", key: `${r.section}:${m.command.id}`, command: m.command, title: m.title, ranges: m.ranges });
      }
      if (r.total > r.matches.length) {
        push(g, { kind: "more", key: `${r.section}:more`, section: r.section, count: r.total - r.matches.length });
      }
      groups.push(g);
    }
    return { groups, flat };
  });

  const activeRow = $derived(view.flat[active] as Row | undefined);
  const textError = $derived.by(() => {
    const s = step;
    if (!s || s.next.kind !== "text" || !s.next.validate) return null;
    try {
      return s.next.validate(query);
    } catch (e) {
      return String(e);
    }
  });

  function disabledReason(c: Command): string | null {
    try {
      return c.disabled?.(ctx) ?? null;
    } catch {
      return null;
    }
  }

  /** Highlighted title segments. */
  function segments(title: string, ranges: readonly (readonly [number, number])[]) {
    const out: { text: string; hit: boolean }[] = [];
    let at = 0;
    for (const [a, b] of ranges) {
      if (a > at) out.push({ text: title.slice(at, a), hit: false });
      out.push({ text: title.slice(a, b), hit: true });
      at = b;
    }
    if (at < title.length) out.push({ text: title.slice(at), hit: false });
    return out;
  }

  // Project items (screens, templates, sources, flows, schedules) while open.
  onMount(() => watchItems());

  onMount(() => {
    returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const d = dialog;
    if (d) {
      if (typeof d.showModal === "function" && !d.open) d.showModal();
      else d.setAttribute("open", "");
    }
    const c = followUp ? getCommand(followUp) : undefined;
    if (c?.next) enterStep(c, t(c.title, safe(() => c.params?.())), c.next, true);
    input?.focus();
    return () => {
      if (!closed) restoreFocus();
    };
  });

  function restoreFocus() {
    if (returnFocus?.isConnected) returnFocus.focus({ preventScroll: true });
  }

  function close() {
    if (closed) return;
    closed = true;
    if (dialog?.open && typeof dialog.close === "function") dialog.close();
    onclose();
    restoreFocus();
  }

  async function select(i: number) {
    const n = view.flat.length;
    if (n === 0) return;
    const next = Math.max(0, Math.min(n - 1, i));
    if (next !== active) armed = null;
    active = next;
    await tick();
    list?.querySelector<HTMLElement>(`[data-index="${active}"]`)?.scrollIntoView?.({ block: "nearest" });
  }

  function move(delta: number) {
    const n = view.flat.length;
    if (n === 0) return;
    void select((active + delta + n) % n);
  }

  /** Tab / Shift+Tab: the first row of the next / previous section (wrapping). */
  function jumpSection(dir: 1 | -1) {
    const gs = view.groups;
    if (gs.length === 0) return;
    let g = gs.findIndex((x) => x.rows.some((r) => r.i === active));
    if (g < 0) g = 0;
    const target = gs[(g + dir + gs.length) % gs.length];
    void select(target.rows[0]?.i ?? 0);
  }

  function enterStep(command: Command, title: string, next: FollowUp, direct = false) {
    step = { command, title, next, mainQuery: query, direct };
    query = next.kind === "text" ? (safe(() => next.value?.(ctx)) ?? "") : "";
    active = 0;
    armed = null;
    void tick().then(() => input?.select());
  }

  function safe<T>(f: () => T): T | undefined {
    try {
      return f();
    } catch {
      return undefined;
    }
  }

  function back() {
    const s = step;
    if (!s) return;
    if (s.direct) return close();
    step = null;
    query = s.mainQuery;
    active = 0;
    armed = null;
  }

  function activate(row: Row | undefined) {
    if (!row) return;
    if (row.kind === "more") {
      const cur = results.find((r) => r.section === row.section)?.matches.length ?? 0;
      limits = { ...limits, [row.section]: cur + MORE_STEP };
      return;
    }
    if (row.kind === "pick") {
      const s = step;
      if (!s) return;
      close();
      void runFollowUp(s.command.id, row.id);
      return;
    }
    const c = row.command;
    if (disabledReason(c)) return;
    // "Show all commands" is the palette's own key: inside the palette it lists every command
    // (the ">" scope, no "Show more" caps) instead of closing and reopening.
    if (c.id === "palette.open") {
      query = ">";
      limits = Object.fromEntries(ALL_SECTIONS.map((s) => [s, Number.MAX_SAFE_INTEGER]));
      active = 0;
      armed = null;
      input?.focus();
      return;
    }
    if (c.next) {
      enterStep(c, row.title, c.next);
      return;
    }
    if (c.confirm && armed !== c.id) {
      armed = c.id;
      return;
    }
    close();
    void runCommand(c.id);
  }

  function submitText() {
    const s = step;
    if (!s || s.next.kind !== "text" || textError) return;
    close();
    void runFollowUp(s.command.id, query);
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.isComposing) return;
    if (armed && e.key !== "Enter") armed = null;
    const textStep = step?.next.kind === "text";
    switch (e.key) {
      case "ArrowDown":
        if (textStep) return;
        e.preventDefault();
        move(1);
        break;
      case "ArrowUp":
        if (textStep) return;
        e.preventDefault();
        move(-1);
        break;
      case "Home":
      case "End":
        if (textStep || e.shiftKey) return;
        e.preventDefault();
        void select(e.key === "Home" ? 0 : view.flat.length - 1);
        break;
      case "Tab":
        e.preventDefault();
        if (!textStep) jumpSection(e.shiftKey ? -1 : 1);
        break;
      case "Enter":
        e.preventDefault();
        if (e.repeat) return;
        if (textStep) submitText();
        else activate(activeRow);
        break;
      case "Escape":
        e.preventDefault();
        e.stopPropagation();
        if (step) back();
        else close();
        break;
      case "Backspace":
        if (step && query === "") {
          e.preventDefault();
          back();
        }
        break;
    }
  }

  function oninput() {
    active = 0;
    armed = null;
    if (!step) limits = {};
  }

  function onCancelEvent(e: Event) {
    e.preventDefault();
    if (step) back();
    else close();
  }

  const placeholder = $derived(step ? t(step.next.placeholder) : t("Type a command or search…"));
  const showList = $derived(!(step && step.next.kind === "text"));
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialog}
  class="palette"
  aria-label={t("Command palette")}
  oncancel={onCancelEvent}
  {@attach backdropClose(() => close())}
>
  <div class="search vf vf-within">
    <Search size={16} aria-hidden="true" />
    {#if step}
      <span class="crumb">{step.title}<ChevronRight size={14} aria-hidden="true" /></span>
    {/if}
    <input
      bind:this={input}
      bind:value={query}
      type="text"
      role={showList ? "combobox" : undefined}
      aria-label={step ? step.title : t("Search commands")}
      aria-expanded={showList ? "true" : undefined}
      aria-controls={showList ? `${uid}-list` : undefined}
      aria-autocomplete={showList ? "list" : undefined}
      aria-activedescendant={showList && activeRow ? `${uid}-o${active}` : undefined}
      aria-invalid={textError ? "true" : undefined}
      aria-describedby={textError ? `${uid}-err` : undefined}
      autocomplete="off"
      spellcheck="false"
      {placeholder}
      {onkeydown}
      {oninput}
    />
  </div>

  {#if !showList}
    {#if textError}<p class="error" id="{uid}-err" role="alert">{textError}</p>{/if}
  {:else}
    <!-- Options never take focus: it stays in the input (aria-activedescendant). -->
    <div
      class="list"
      id="{uid}-list"
      role="listbox"
      tabindex="-1"
      aria-label={step ? step.title : t("Commands")}
      bind:this={list}
      onmousedown={(e) => e.preventDefault()}
    >
      {#each view.groups as g, gi (g.section ?? gi)}
        <div class="group" role="group" aria-labelledby="{uid}-g{gi}">
          <div class="section-label head" id="{uid}-g{gi}" role="presentation">{g.label}</div>
          {#each g.rows as { row, i } (row.key)}
            {@const selected = i === active}
            {#if row.kind === "more"}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <div
                id="{uid}-o{i}"
                class="row more"
                role="option"
                tabindex="-1"
                data-index={i}
                aria-selected={selected}
                onclick={() => activate(row)}
                onpointermove={() => {
                  if (!selected) void select(i);
                }}
              >
                <span class="dot" aria-hidden="true"></span>
                <span class="icon" aria-hidden="true"></span>
                <span class="title">{t("Show {count} more", { count: row.count })}</span>
              </div>
            {:else}
              {@const reason = row.kind === "cmd" ? disabledReason(row.command) : null}
              {@const isArmed = row.kind === "cmd" && armed === row.command.id}
              {@const keys = row.kind === "cmd" ? keysFor(row.command.id) : []}
              {@const Icon = row.kind === "cmd" ? row.command.icon : undefined}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <div
                id="{uid}-o{i}"
                class="row"
                class:armed={isArmed}
                role="option"
                tabindex="-1"
                data-index={i}
                aria-selected={selected}
                aria-disabled={reason ? "true" : undefined}
                title={reason ?? undefined}
                onclick={() => {
                  if (!selected) void select(i);
                  activate(row);
                }}
                onpointermove={() => {
                  if (!selected) void select(i);
                }}
              >
                <span class="dot" aria-hidden="true"></span>
                <span class="icon" aria-hidden="true">{#if Icon}<Icon size={16} />{/if}</span>
                <span class="title"
                  >{#each segments(row.title, row.ranges) as seg, si (si)}{#if seg.hit}<mark>{seg.text}</mark
                      >{:else}{seg.text}{/if}{/each}</span
                >
                {#if isArmed}
                  <span class="aside confirm" role="status">{t("Press Enter again to confirm")}</span>
                {:else if reason}
                  <span class="aside reason">{reason}</span>
                {:else if row.kind === "pick" && row.detail}
                  <span class="aside">{row.detail}</span>
                {:else if keys.length > 0}
                  <span class="aside keys">
                    {#each keys.slice(0, 2) as k (k)}<kbd class="mono">{k}</kbd>{/each}
                  </span>
                {/if}
              </div>
            {/if}
          {/each}
        </div>
      {:else}
        <p class="none" role="presentation">{t("No matches")}</p>
      {/each}
    </div>
  {/if}
</dialog>

<style>
  .palette {
    width: min(640px, calc(100vw - 2 * var(--s-4)));
    max-height: min(520px, calc(100vh - 2 * var(--s-8)));
    margin: 12vh auto auto;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    color: var(--text);
    overflow: hidden;
  }
  .palette[open] {
    display: flex;
    flex-direction: column;
    animation: enter 140ms var(--ease);
  }
  .palette::backdrop {
    background: var(--overlay);
  }
  @media (prefers-reduced-motion: reduce) {
    .palette[open] {
      animation: none;
    }
  }
  /* Slides only, never fades: opaque from its first frame, which a busy first load can hold for
     seconds (a fade then showed the page, on-air tally lines too, through the dialog). */
  @keyframes enter {
    from {
      transform: translateY(-8px);
    }
  }

  .search {
    --vf-radius: var(--r-control);
    display: flex;
    align-items: center;
    gap: var(--s-2);
    margin: var(--s-2);
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text-muted);
  }
  .search input {
    flex: 1;
    min-width: 0;
    height: var(--control-h);
    border: 0;
    background: transparent;
    color: var(--text);
    outline: none;
  }
  .search input::placeholder {
    color: var(--text-muted);
  }
  .crumb {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    flex: none;
    max-width: 50%;
    overflow: hidden;
    color: var(--text);
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .list {
    min-height: 0;
    padding: 0 var(--s-2) var(--s-2);
    overflow: auto;
    overscroll-behavior: contain;
  }
  .head {
    padding: var(--s-2) var(--s-2) var(--s-1);
  }
  .row {
    display: grid;
    grid-template-columns: 6px 16px minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--s-2);
    min-height: var(--row-h);
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    cursor: pointer;
  }
  .row[aria-selected="true"] {
    background: var(--selected-bg);
    color: var(--accent-text);
  }
  .row[aria-selected="true"] .dot {
    width: 6px;
    height: 6px;
    border-radius: var(--r-full);
    background: currentColor;
  }
  .row[aria-disabled="true"] {
    color: var(--text-muted);
    cursor: default;
  }
  .row.armed,
  .row.armed[aria-selected="true"] {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .icon {
    display: inline-flex;
    color: var(--text-muted);
  }
  .row[aria-selected="true"] .icon,
  .row.armed .icon {
    color: inherit;
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  mark {
    background: none;
    color: var(--accent-text);
    font-weight: 600;
  }
  .row.armed mark {
    color: inherit;
  }
  .more .title {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .more[aria-selected="true"] .title {
    color: var(--accent-text);
  }
  .aside {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .confirm {
    color: var(--danger);
  }
  .keys {
    display: inline-flex;
    gap: var(--s-1);
  }
  kbd {
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text-muted);
    font-size: var(--fs-xs);
    line-height: 18px;
  }
  .none,
  .error {
    margin: 0;
    padding: var(--s-2) var(--s-4) var(--s-3);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .error {
    color: var(--danger);
  }
</style>

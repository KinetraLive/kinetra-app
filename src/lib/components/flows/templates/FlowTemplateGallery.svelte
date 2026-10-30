<!--
  FlowTemplateGallery (lazy chunk): "New flow from a template", opened from /flows (New flow →
  From a template, the empty state). A native modal <dialog> in two steps:

  1. The gallery: the built-in flow templates grouped by category (Show running, Event schedule,
     Sponsors, Sports & LAN, Building blocks), filtered by the search box. Each card: name, what it
     does, its chain of nodes, "Adds N flows", and "Starts disarmed" / "Partly disarmed" when it
     has flows with automatic triggers (those are added disarmed; the form says which).
  2. "Point it at": one picker per slot (screens, layers as "Screen · Layer", a layer's timer or
     other fields, text sources, schedules, flows). Smart defaults pick a layer made from the
     slot's preferred preset, else the first choice for a required slot. Add stays disabled with
     the reason until every required slot is filled; optional slots can stay empty. Add creates the
     flows (`addFlowTemplate`), says "Added <name>", closes and hands the flows to the parent. A
     failed add shows the error inline and keeps the dialog open.

  Keyboard: the cards are one listbox (arrows move, Home/End, Enter/Space choose); ArrowDown in the
  search box moves into it. Focus goes to the search box on open, to the first picker after
  choosing, back to the card on Back, and to the opener on close.

  Props:
    onclose: () => void               Esc, Cancel, a backdrop click, or after Add
    onadded: (flows: FlowInfo[]) => void   the created flows, in order
-->
<script lang="ts">
  import { backdropClose } from "$lib/components/backdrop";
  import { onMount, tick } from "svelte";
  import ArrowLeft from "lucide-svelte/icons/arrow-left";
  import Search from "lucide-svelte/icons/search";
  import { toAppError } from "$lib/api";
  import { addFlowTemplate, listFlowTemplates } from "$lib/api/flows";
  import Button from "$lib/components/Button.svelte";
  import Select from "$lib/components/Select.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { FLOW_NAME_MAX, type FlowInfo, type FlowSlot, type FlowTemplateInfo } from "$lib/types";
  import { nameError } from "../names";
  import {
    NO_TARGETS,
    emptyLabel,
    filledSlots,
    groupTemplates,
    missingSlots,
    slotDefaults,
    slotOptions,
    armHint,
    armNote,
    type Targets,
  } from "./slots";
  import { loadTargets } from "./targets";

  let { onclose, onadded }: { onclose: () => void; onadded: (flows: FlowInfo[]) => void } = $props();

  const uid = $props.id();
  let dialog: HTMLDialogElement | undefined = $state();
  let searchInput: HTMLInputElement | undefined = $state();
  let listbox: HTMLDivElement | undefined = $state();
  let form: HTMLDivElement | undefined = $state();

  let templates = $state.raw<FlowTemplateInfo[]>([]);
  let loaded = $state(false);
  let loadError = $state<string | null>(null);
  let targets = $state.raw<Targets>(NO_TARGETS);
  const targetsReady = loadTargets().then((t) => (targets = t));

  let query = $state("");
  let active = $state(0);
  let chosen = $state.raw<FlowTemplateInfo | null>(null);
  let values = $state<Record<string, string>>({});
  let name = $state("");
  let adding = $state(false);
  let error = $state<string | null>(null);

  const groups = $derived(groupTemplates(templates, query));
  const flat = $derived(groups.flatMap((g) => g.items));
  const tabStop = $derived(Math.min(active, Math.max(0, flat.length - 1)));
  const missing = $derived(chosen ? missingSlots(chosen.slots, values) : []);
  const nameInvalid = $derived(nameError(name));
  const reason = $derived(
    missing.length > 0 ? `Choose ${listed(missing.map((s) => s.label))} to add it.` : nameInvalid,
  );
  const note = $derived(chosen ? armNote(chosen, name.trim() || chosen.name) : null);

  function listed(labels: string[]): string {
    if (labels.length <= 1) return labels[0] ?? "";
    return `${labels.slice(0, -1).join(", ")} and ${labels[labels.length - 1]}`;
  }

  onMount(() => {
    const d = dialog;
    let alive = true;
    listFlowTemplates().then(
      (list) => {
        if (!alive) return;
        templates = list ?? [];
        loaded = true;
      },
      (e: unknown) => {
        if (!alive) return;
        loadError = toAppError(e).message;
        loaded = true;
      },
    );
    if (!d) return;
    const back = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (typeof d.showModal === "function") d.showModal();
    else d.setAttribute("open", "");
    void tick().then(() => searchInput?.focus());
    return () => {
      alive = false;
      if (typeof d.close === "function") d.close();
      else d.removeAttribute("open");
      if (back?.isConnected) back.focus();
    };
  });

  function cancel() {
    if (!adding) onclose();
  }

  // ---- Gallery ----

  function cards(): HTMLElement[] {
    return [...(listbox?.querySelectorAll<HTMLElement>('[role="option"]') ?? [])];
  }

  function focusCard(i: number) {
    active = i;
    void tick().then(() => cards()[i]?.focus());
  }

  /** The card in the row above or below (by position), nearest horizontally; ±1 without layout. */
  function vertical(from: number, dir: 1 | -1): number {
    const els = cards();
    const at = els[from]?.getBoundingClientRect();
    if (!at) return from;
    let best = -1;
    let bestRow = Infinity;
    let bestX = Infinity;
    els.forEach((el, i) => {
      const r = el.getBoundingClientRect();
      const dy = (r.top - at.top) * dir;
      if (dy <= 1) return;
      const dx = Math.abs(r.left - at.left);
      if (dy < bestRow - 1 || (Math.abs(dy - bestRow) <= 1 && dx < bestX)) {
        best = i;
        bestRow = dy;
        bestX = dx;
      }
    });
    if (best >= 0) return best;
    return Math.max(0, Math.min(els.length - 1, from + dir));
  }

  function onListKeydown(e: KeyboardEvent) {
    const n = flat.length;
    if (n === 0) return;
    let i: number | null = null;
    if (e.key === "ArrowRight") i = Math.min(n - 1, tabStop + 1);
    else if (e.key === "ArrowLeft") i = Math.max(0, tabStop - 1);
    else if (e.key === "ArrowDown") i = vertical(tabStop, 1);
    else if (e.key === "ArrowUp") i = vertical(tabStop, -1);
    else if (e.key === "Home") i = 0;
    else if (e.key === "End") i = n - 1;
    else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      const t = flat[tabStop];
      if (t) void choose(t);
      return;
    }
    if (i === null) return;
    e.preventDefault();
    focusCard(i);
  }

  function onSearchKeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown" && flat.length > 0) {
      e.preventDefault();
      focusCard(tabStop);
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (flat.length === 1) void choose(flat[0]);
      else if (flat.length > 0) focusCard(tabStop);
    }
  }

  async function choose(t: FlowTemplateInfo) {
    active = Math.max(0, flat.indexOf(t));
    await targetsReady;
    values = slotDefaults(t.slots, targets);
    name = t.name;
    error = null;
    chosen = t;
    await tick();
    form?.querySelector<HTMLElement>("select, input")?.focus();
  }

  async function back() {
    chosen = null;
    error = null;
    focusCard(tabStop);
  }

  // ---- Form ----

  function optionsFor(slot: FlowSlot) {
    const opts = slotOptions(slot, targets).map(({ value, label }) => ({ value, label }));
    const empty = { value: "", label: emptyLabel(slot, opts.length), disabled: !slot.optional };
    return [empty, ...opts];
  }

  async function add(e?: SubmitEvent) {
    e?.preventDefault();
    const t = chosen;
    if (!t || adding || reason) return;
    adding = true;
    error = null;
    const label = name.trim();
    let flows: FlowInfo[];
    try {
      flows = await addFlowTemplate({ templateId: t.id, name: label, slots: filledSlots(t.slots, values) });
    } catch (err) {
      error = toAppError(err).message;
      adding = false;
      return;
    }
    adding = false;
    toasts.push({ kind: "success", message: `Added ${label}` });
    onclose();
    onadded(flows ?? []);
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialog}
  class="dialog"
  aria-labelledby="{uid}-title"
  oncancel={(e) => {
    e.preventDefault();
    cancel();
  }}
  {@attach backdropClose(cancel)}
>
  {#if !chosen}
    <div class="inner">
      <div class="top">
        <h2 id="{uid}-title" class="title">New flow from a template</h2>
        <div class="search vf vf-within">
          <Search size={14} aria-hidden="true" />
          <input
            bind:this={searchInput}
            bind:value={query}
            type="search"
            placeholder="Search templates"
            aria-label="Search templates"
            aria-controls="{uid}-list"
            autocomplete="off"
            spellcheck="false"
            oninput={() => (active = 0)}
            onkeydown={onSearchKeydown}
          />
        </div>
      </div>

      {#if loadError}
        <p class="error" role="alert">Couldn't load the templates: {loadError}</p>
      {/if}
      <div
        id="{uid}-list"
        class="groups"
        role="listbox"
        tabindex="-1"
        aria-label="Flow templates"
        bind:this={listbox}
        onkeydown={onListKeydown}
      >
        {#each groups as g (g.id)}
          <div class="group" role="group" aria-labelledby="{uid}-g-{g.id}">
            <h3 class="section-label" id="{uid}-g-{g.id}">{g.label}</h3>
            <div class="cards">
              {#each g.items as t (t.id)}
                {@const i = flat.indexOf(t)}
                {@const hint = armHint(t.id)}
                <div
                  class="card vf"
                  role="option"
                  aria-selected="false"
                  tabindex={i === tabStop ? 0 : -1}
                  data-flow-template={t.id}
                  onclick={() => void choose(t)}
                  onfocus={() => (active = i)}
                >
                  <span class="name">{t.name}</span>
                  <span class="desc">{t.description}</span>
                  <span class="chain">{t.chain}</span>
                  {#if t.flows > 1 || hint}
                    <span class="notes">
                      {#if t.flows > 1}<span>Adds {t.flows} flows</span>{/if}
                      {#if hint}<span>{hint}</span>{/if}
                    </span>
                  {/if}
                </div>
              {/each}
            </div>
          </div>
        {/each}
      </div>
      {#if loaded && !loadError && flat.length === 0}
        <p class="none">
          {templates.length === 0 ? "No flow templates are available." : `No templates match “${query.trim()}”.`}
          {#if templates.length > 0}
            <button type="button" class="link vf" onclick={() => (query = "")}>Clear the search</button>
          {/if}
        </p>
      {/if}

      <div class="actions">
        <Button variant="ghost" onclick={cancel}>Cancel</Button>
      </div>
    </div>
  {:else}
    <form class="inner" onsubmit={add} novalidate>
      <div class="form-head">
        <Button variant="ghost" size="sm" onclick={back} disabled={adding}>
          {#snippet icon()}<ArrowLeft size={14} />{/snippet}
          Back
        </Button>
      </div>
      <div class="about">
        <h2 id="{uid}-title" class="title">{chosen.name}</h2>
        <p class="desc">{chosen.description}</p>
        <p class="chain">{chosen.chain}</p>
        {#if chosen.flows > 1}
          <p class="note">Adds {chosen.flows} flows, named “{name.trim() || chosen.name} · …”.</p>
        {/if}
        {#if note}<p class="note">{note}</p>{/if}
      </div>

      <div class="fields" bind:this={form}>
        {#if chosen.slots.length > 0}
          <div class="point">
            <h3 class="section-label">Point it at</h3>
            {#if chosen.slots.some((s) => !s.optional)}<span class="legend" aria-hidden="true">* Required</span>{/if}
          </div>
          <div class="grid">
            {#each chosen.slots as slot (slot.key)}
              <div class="field" data-slot={slot.key}>
                <label for="{uid}-slot-{slot.key}">
                  {slot.label}{#if !slot.optional}<span class="req" aria-hidden="true"> *</span>{/if}
                  {#if slot.optional}<span class="opt"> · Optional</span>{/if}
                </label>
                <Select
                  id="{uid}-slot-{slot.key}"
                  aria-required={!slot.optional}
                  disabled={adding}
                  value={values[slot.key] ?? ""}
                  options={optionsFor(slot)}
                  onchange={(v) => (values[slot.key] = v)}
                />
              </div>
            {/each}
          </div>
        {/if}
        <div class="field name-field">
          <label for="{uid}-name">Name</label>
          <div class="input vf vf-within">
            <input
              id="{uid}-name"
              type="text"
              bind:value={name}
              maxlength={FLOW_NAME_MAX}
              autocomplete="off"
              spellcheck="false"
              aria-invalid={nameInvalid !== null}
              disabled={adding}
            />
          </div>
        </div>
      </div>

      {#if error}<p class="error" role="alert">{error}</p>{/if}
      <div class="actions">
        {#if reason}<p class="reason" id="{uid}-reason">{reason}</p>{/if}
        <Button variant="ghost" onclick={cancel} disabled={adding}>Cancel</Button>
        <Button
          variant="primary"
          type="submit"
          disabled={reason !== null && !adding}
          loading={adding}
          aria-describedby={reason ? `${uid}-reason` : undefined}
        >
          Add
        </Button>
      </div>
    </form>
  {/if}
</dialog>

<style>
  .dialog {
    width: min(920px, calc(100vw - 2 * var(--s-6)));
    max-height: calc(100vh - 2 * var(--s-6));
    margin: auto;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    color: var(--text);
    overflow: auto;
  }
  .dialog[open] {
    display: flex;
    flex-direction: column;
    animation: enter var(--dur) var(--ease);
  }
  .dialog::backdrop {
    background: var(--overlay);
    animation: fade var(--dur) var(--ease);
  }
  .inner {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--s-4);
    min-height: 0;
    padding: var(--s-5);
  }
  .top {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
  }
  .title {
    margin: 0;
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .search {
    --vf-radius: var(--r-control);
    display: flex;
    align-items: center;
    gap: var(--s-2);
    width: min(280px, 100%);
    height: var(--control-h);
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text-muted);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    color: var(--text);
  }
  .groups {
    display: flex;
    flex-direction: column;
    gap: var(--s-4);
    min-height: 0;
    margin: calc(-1 * var(--s-2));
    padding: var(--s-2);
    overflow: auto;
  }
  .groups:focus-visible {
    outline: none;
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }
  .group h3 {
    margin: 0;
    color: var(--text-muted);
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: var(--s-3);
  }
  .card {
    --vf-radius: var(--r-panel);
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    min-width: 0;
    padding: var(--s-3);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
    cursor: pointer;
    user-select: none;
  }
  .card:hover {
    background: var(--surface-3);
  }
  .card:focus-visible {
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .name {
    font-weight: 600;
  }
  .desc {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .chain {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-xs);
    letter-spacing: 0.01em;
  }
  .card .chain {
    margin-top: auto;
    padding-top: var(--s-1);
  }
  .notes {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-1) var(--s-3);
    color: var(--accent-text);
    font-size: var(--fs-xs);
  }
  .none {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .link {
    --vf-radius: var(--r-control);
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--accent-text);
    text-decoration: underline;
  }
  .form-head {
    display: flex;
    margin: calc(-1 * var(--s-2)) 0 0 calc(-1 * var(--s-2));
  }
  .about {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
  }
  .about .desc {
    font-size: var(--fs-md);
  }
  .note {
    margin: var(--s-1) 0 0;
    color: var(--accent-text);
    font-size: var(--fs-sm);
  }
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding-top: var(--s-4);
    border-top: 1px solid var(--line);
  }
  .point {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--s-2);
  }
  .point h3 {
    margin: 0;
    color: var(--text-muted);
  }
  .legend,
  .opt {
    color: var(--text-muted);
    font-size: var(--fs-xs);
    font-weight: 400;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: var(--s-3) var(--s-4);
  }
  .field {
    display: grid;
    gap: var(--s-1);
    min-width: 0;
  }
  .name-field {
    max-width: 420px;
  }
  label {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .req {
    color: var(--text);
  }
  .input {
    --vf-radius: var(--r-control);
    display: flex;
    height: var(--control-h);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .input input {
    flex: 1;
    min-width: 0;
    padding: 0 var(--s-2);
    border: 0;
    background: transparent;
    color: var(--text);
  }
  .error {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--s-2);
  }
  .reason {
    margin: 0 auto 0 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  /* Slides only, never fades: opaque from its first frame, which a busy first load can hold for
     seconds (a fade then showed the page, on-air tally lines too, through the dialog). */
  @keyframes enter {
    from {
      transform: translateY(8px);
    }
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .dialog[open],
    .dialog::backdrop {
      animation: none;
    }
  }
</style>

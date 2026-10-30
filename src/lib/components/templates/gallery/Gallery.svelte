<!--
  "New template" gallery: a native modal <dialog> listing Blank and the built-in presets. Choose
  one, name it, Create: the template is created, the gallery closes and the editor opens
  (`/templates/<id>`). Choosing a card (click, Enter, Space) fills Name with a default (the
  preset's name; "Untitled template" for Blank) and focuses it, so Enter creates; a double-click
  creates at once with that name.

  Fits the window: the title, the style chips and the footer (Name, Create) stay put; only the
  cards scroll.

  Lazy chunk: open it with `GalleryLauncher` / `loadGallery()` from ./launcher.svelte.ts.

  Props:
    open: boolean          shows the dialog (showModal); false closes it
    layer?: GalleryLayer   opened from a layer's "Browse styles…": the title names the layer, only
                           its template's card shows, and the footer offers "Apply to project" and
                           "Apply to this layer only" instead of Name / Create
    onclose: () => void    Esc, Cancel, a backdrop click, after Create or an Apply. The parent sets
                           open=false.

  Layout:
    style chips    All + every style some preset ships (`list_styles`, Classic too). A style shows only the presets
                   that ship that look, their thumbnails in it, and "Apply <style> to project"
                   (the apply flow in $lib/components/styles/apply: confirm when layers on air
                   would switch, a result toast with Undo). Creating with a style chosen makes
                   that look the new template's default.
    library        one card per template type (PresetInfo.kind), with the styles it ships
    More           the other built-in presets
    Legacy         the four older lower-third looks, collapsed until opened

  Thumbnails are static images (`presets/thumbs/<id>-<style>.png`, else `<id>.png`), loaded
  lazily. Hovering or focusing a card plays ONE live preview of that look over its thumbnail
  (./preview.ts, a lazy chunk: the renderer's own compile with the preset's sample data, no
  iframe), released when the pointer and focus leave. The checkerboard is the fixed dark one
  (`.checker-dark`) in both themes: broadcast graphics are judged over dark video.

  Focus: the first card is focused on open; focus returns to whatever had it on close. Each
  section is a listbox with one tab stop (arrow keys / Home / End move).
-->
<script lang="ts">
  import { tick } from "svelte";
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import { goto } from "$app/navigation";
  import { toAppError } from "$lib/api";
  import {
    createTemplate,
    getTemplate,
    listPresets,
    listStyles,
    setDefaultTemplateVariant,
  } from "$lib/api/templates";
  import Button from "$lib/components/Button.svelte";
  import { offered } from "$lib/components/styles/styles";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { PresetInfo, StyleInfo, TemplateSource } from "$lib/types";
  import { NAME_MAX, gridColumns, moveIndex, validName } from "../list/format";
  import type { GalleryLayer } from "./launcher.svelte";
  import { variantThumbUrl } from "./thumbs";

  let { open, layer = null, onclose }: { open: boolean; layer?: GalleryLayer | null; onclose: () => void } =
    $props();

  type Choice = {
    key: string;
    name: string;
    description: string;
    source: TemplateSource;
    preset: PresetInfo | null;
  };
  type Section = { key: "library" | "more" | "legacy"; label: string | null; choices: Choice[] };

  const ALL = "all";
  const BLANK: Choice = {
    key: "blank",
    name: "Blank",
    description: "An empty template: your own HTML, CSS and fields.",
    source: { kind: "blank" },
    preset: null,
  };
  const BLANK_NAME = "Untitled template";

  const uid = $props.id();
  let dialog: HTMLDialogElement | undefined = $state();
  let body: HTMLDivElement | undefined = $state();
  let nameInput: HTMLInputElement | undefined = $state();
  let presets = $state.raw<PresetInfo[]>([]);
  let styles = $state.raw<StyleInfo[]>([]);
  /** Thumbnail URL per `<presetId>:<variant>`: missing while resolving, null when there is none. */
  let thumbs = $state.raw<Record<string, string | null>>({});
  /** The style chip: ALL or a style id. */
  let style = $state(ALL);
  let legacyOpen = $state(false);
  let chosen = $state<string | null>(null);
  /** Roving tab stop per section while nothing in it is chosen. */
  let active = $state<Record<string, number>>({});
  let name = $state("");
  let creating = $state(false);
  let applying = $state(false);
  /** The card whose live preview shows (hover or focus). */
  let hovered = $state<string | null>(null);
  /** Layer mode: the preset the layer's template was made from (null: unknown / none). */
  let layerPreset = $state<string | null>(null);
  let returnFocus: HTMLElement | null = null;
  let previewMod: Promise<typeof import("./preview")> | null = null;

  const choiceOf = (p: PresetInfo): Choice => ({
    key: p.id,
    name: p.name,
    description: p.description,
    source: { kind: "preset", presetId: p.id },
    preset: p,
  });
  const ships = (p: PresetInfo, s: string) => p.variants?.some((v) => v.id === s) ?? false;
  const matches = (p: PresetInfo) => style === ALL || ships(p, style);
  /** The look a card shows: the chosen style, else the preset's default (first) variant. */
  const lookOf = (p: PresetInfo) => (style !== ALL && ships(p, style) ? style : (p.variants?.[0]?.id ?? ""));

  const styleName = $derived(styles.find((s) => s.id === style)?.name ?? "");
  const sections = $derived.by<Section[]>(() => {
    const library = presets.filter((p) => p.kind && !p.legacy);
    if (layer) {
      const own = library.filter((p) => p.id === layerPreset);
      return [{ key: "library", label: null, choices: (own.length ? own : library).filter(matches).map(choiceOf) }];
    }
    const out: Section[] = [
      {
        key: "library",
        label: null,
        choices: [...(style === ALL ? [BLANK] : []), ...library.filter(matches).map(choiceOf)],
      },
      { key: "more", label: "More", choices: presets.filter((p) => !p.kind && !p.legacy && matches(p)).map(choiceOf) },
      { key: "legacy", label: "Legacy", choices: presets.filter((p) => p.legacy && matches(p)).map(choiceOf) },
    ];
    return out.filter((s) => s.choices.length > 0);
  });
  const allChoices = $derived(sections.flatMap((s) => s.choices));
  const choice = $derived(allChoices.find((c) => c.key === chosen) ?? null);
  const canCreate = $derived(choice !== null && validName(name) && !creating);

  const tabStop = (s: Section) => {
    const i = s.choices.findIndex((c) => c.key === chosen);
    return i >= 0 ? i : Math.min(active[s.key] ?? 0, s.choices.length - 1);
  };

  // Presets and styles, once per mount.
  $effect(() => {
    let alive = true;
    Promise.all([listPresets(), listStyles().catch(() => [] as StyleInfo[])]).then(
      ([list, st]) => {
        if (!alive) return;
        presets = list;
        styles = offered(st, list);
      },
      (e: unknown) => {
        if (alive)
          toasts.push({ kind: "error", message: `Couldn't load the gallery: ${toAppError(e).message}` });
      },
    );
    return () => {
      alive = false;
    };
  });

  // Layer mode: which preset the layer's template came from (refetched only when it changes).
  const layerTemplate = $derived(layer?.templateId ?? null);
  $effect(() => {
    const id = layerTemplate;
    layerPreset = null;
    if (!id) return;
    let alive = true;
    getTemplate(id).then(
      (t) => {
        if (alive) layerPreset = t.preset?.id ?? null;
      },
      () => {},
    );
    return () => {
      alive = false;
    };
  });

  // Thumbnails of the looks on show, resolved once each.
  $effect(() => {
    for (const c of allChoices) {
      if (!c.preset) continue;
      const p = c.preset;
      const look = lookOf(p);
      const key = `${p.id}:${look}`;
      if (key in thumbs) continue;
      thumbs = { ...thumbs, [key]: null };
      void variantThumbUrl(p.id, look).then((url) => {
        if (url !== null) thumbs = { ...thumbs, [key]: url };
      });
    }
  });

  // Open / close the native dialog; focus the first card on open, give focus back on close.
  $effect(() => {
    const d = dialog;
    if (!d) return;
    if (open && !d.open) {
      returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      chosen = null;
      active = {};
      name = "";
      style = ALL;
      legacyOpen = false;
      if (typeof d.showModal === "function") d.showModal();
      else d.setAttribute("open", "");
      void tick().then(() => body?.querySelector<HTMLElement>('[role="option"]')?.focus());
    } else if (!open && d.open) {
      shut(d);
    }
  });

  // Unmounted while open: still hand focus back, and free the preview.
  $effect(() => () => {
    if (dialog?.open) shut(dialog);
    void previewMod?.then((m) => m.hidePreview());
  });

  function shut(d: HTMLDialogElement) {
    hovered = null;
    if (typeof d.close === "function") d.close();
    else d.removeAttribute("open");
    const el = returnFocus;
    returnFocus = null;
    if (el?.isConnected) el.focus();
  }

  function cancel() {
    if (!creating && !applying) onclose();
  }

  function onCancelEvent(e: Event) {
    // The parent owns visibility; it closes us from onclose.
    e.preventDefault();
    cancel();
  }

  function onBackdropClick(e: MouseEvent) {
    if (e.target === dialog) cancel();
  }

  /** Chooses a card; `toName` (a click, Enter or Space, never an arrow key) then focuses Name. */
  function pick(c: Choice, toName = false) {
    if (chosen !== c.key) {
      chosen = c.key;
      if (!layer) name = c.key === BLANK.key ? BLANK_NAME : c.name;
    }
    if (toName && !layer) {
      void tick().then(() => {
        nameInput?.focus();
        nameInput?.select();
      });
    }
  }

  function chooseStyle(s: string) {
    style = s;
    // A chosen card the style hides is no longer chosen.
    if (chosen !== null && !allChoices.some((c) => c.key === chosen)) chosen = null;
  }

  function focusChoice(section: string, i: number) {
    void tick().then(() =>
      body?.querySelectorAll<HTMLElement>(`[data-section="${section}"] [role="option"]`)[i]?.focus(),
    );
  }

  function onGridKeydown(e: KeyboardEvent, s: Section) {
    const grid = e.currentTarget instanceof HTMLElement ? e.currentTarget : undefined;
    const i = moveIndex(e.key, tabStop(s), s.choices.length, gridColumns(grid));
    if (i === null) return;
    e.preventDefault();
    if (chosen !== null && s.choices.some((c) => c.key === chosen)) pick(s.choices[i]);
    else active = { ...active, [s.key]: i };
    focusChoice(s.key, i);
  }

  function onChoiceKeydown(e: KeyboardEvent, c: Choice) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      pick(c, true);
    }
  }

  /** Attachment on the live preview box of the hovered / focused card. */
  function livePreview(p: PresetInfo, look: string) {
    return (node: HTMLElement) => {
      previewMod ??= import("./preview");
      const mod = previewMod;
      void mod.then((m) => m.showPreview(node, p.id, look || "classic"));
      return () => void mod.then((m) => m.hidePreview(node));
    };
  }

  /** Pointer or focus left a card: its preview goes, unless the card still has the other. */
  function leave(c: Choice, e: Event) {
    const card = e.currentTarget;
    if (!(card instanceof HTMLElement)) return;
    if (e.type === "blur") {
      if (card.matches(":hover")) return;
    } else if (card.matches(":focus-within")) {
      return;
    }
    if (hovered === c.key) hovered = null;
  }

  async function create(e?: SubmitEvent) {
    e?.preventDefault();
    if (layer || !canCreate || !choice) return;
    creating = true;
    const look = style !== ALL && choice.preset && ships(choice.preset, style) ? style : null;
    let id: string;
    try {
      const doc = await createTemplate(name.trim(), choice.source);
      id = doc.id;
      if (look && doc.defaultVariant !== look) await setDefaultTemplateVariant(id, look).catch(() => {});
    } catch (err) {
      toasts.push({
        kind: "error",
        message: `Couldn't create the template: ${toAppError(err).message}`,
      });
      return;
    } finally {
      creating = false;
    }
    onclose();
    await goto(`/templates/${id}`);
  }

  async function apply(scope: "project" | "layer") {
    const s = styles.find((x) => x.id === style);
    if (!s || applying) return;
    applying = true;
    try {
      const m = await import("$lib/components/styles/apply");
      const r =
        scope === "layer" && layer
          ? await m.applyToLayer(s, { screenId: layer.screenId, layerId: layer.layerId, name: layer.name })
          : await m.applyToProject(s);
      if (r) {
        applying = false;
        onclose();
      }
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't apply ${s.name}: ${toAppError(e).message}` });
    } finally {
      applying = false;
    }
  }
</script>

{#snippet card(c: Choice, s: Section, i: number)}
  {@const p = c.preset}
  {@const look = p ? lookOf(p) : ""}
  {@const thumb = p ? thumbs[`${p.id}:${look}`] : null}
  <div
    class="choice vf"
    role="option"
    aria-selected={chosen === c.key}
    tabindex={i === tabStop(s) ? 0 : -1}
    data-choice={c.key}
    onclick={() => pick(c, true)}
    ondblclick={() => {
      pick(c);
      void create();
    }}
    onkeydown={(e) => onChoiceKeydown(e, c)}
    onpointerenter={() => p && (hovered = c.key)}
    onpointerleave={(e) => leave(c, e)}
    onfocus={() => p && (hovered = c.key)}
    onblur={(e) => leave(c, e)}
  >
    <span class="thumb checker-dark">
      {#if thumb}
        <img src={thumb} alt="" loading="lazy" decoding="async" width="320" height="180" />
      {:else}
        <span class="thumb-name" aria-hidden="true">{c.name}</span>
      {/if}
      {#if p && hovered === c.key}
        {#key look}
          <span class="live checker-dark" data-preview={c.key} {@attach livePreview(p, look)}></span>
        {/key}
      {/if}
    </span>
    <span class="name">{c.name}</span>
    <span class="desc">{c.description}</span>
    {#if p?.kind && p.variants?.length}
      <span class="styles" aria-label="Styles: {p.variants.map((v) => v.name).join(', ')}">
        {#each p.variants as v (v.id)}
          <span class="style" class:on={v.id === look}>{v.name}</span>
        {/each}
      </span>
    {/if}
  </div>
{/snippet}

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialog}
  class="dialog"
  aria-labelledby="{uid}-title"
  oncancel={onCancelEvent}
  onclick={onBackdropClick}
>
  <form class="inner" onsubmit={create} novalidate>
    <h2 id="{uid}-title" class="title">
      {#if layer}Styles for “{layer.name}”{:else}New template{/if}
    </h2>

    {#if styles.length > 0}
      <div class="bar">
        <div class="chips" role="group" aria-label="Style">
          <button type="button" class="chip vf" aria-pressed={style === ALL} onclick={() => chooseStyle(ALL)}>All</button>
          {#each styles as s (s.id)}
            <button
              type="button"
              class="chip vf"
              aria-pressed={style === s.id}
              title={s.description}
              onclick={() => chooseStyle(s.id)}>{s.name}</button
            >
          {/each}
        </div>
        {#if style !== ALL && !layer}
          <Button size="sm" disabled={applying} loading={applying} onclick={() => void apply("project")}>
            Apply {styleName} to project
          </Button>
        {/if}
      </div>
    {/if}

    <div class="body" bind:this={body}>
      {#each sections as s (s.key)}
        {#if s.key === "legacy"}
          <button
            type="button"
            class="disclosure section-label vf"
            aria-expanded={legacyOpen}
            aria-controls="{uid}-legacy"
            onclick={() => (legacyOpen = !legacyOpen)}
          >
            <span class="chev" aria-hidden="true"><ChevronRight size={14} /></span>
            Legacy <span class="count">{s.choices.length}</span>
          </button>
        {:else if s.label}
          <h3 class="section-label" id="{uid}-{s.key}-h">{s.label}</h3>
        {/if}
        {#if s.key !== "legacy" || legacyOpen}
          <div
            class="choices"
            role="listbox"
            tabindex="-1"
            id={s.key === "legacy" ? `${uid}-legacy` : undefined}
            data-section={s.key}
            aria-label={s.key === "library" ? (layer ? "Template" : "Start from") : s.key === "legacy" ? "Legacy" : undefined}
            aria-labelledby={s.key === "more" ? `${uid}-more-h` : undefined}
            onkeydown={(e) => onGridKeydown(e, s)}
          >
            {#each s.choices as c, i (c.key)}
              {@render card(c, s, i)}
            {/each}
          </div>
        {/if}
      {/each}
    </div>

    <div class="foot">
      {#if layer}
        <p class="hint" id="{uid}-hint">
          {#if style === ALL}Choose a style above.{:else}On air, the change follows Live edit.{/if}
        </p>
        <div class="actions">
          <Button variant="ghost" onclick={cancel} disabled={applying}>Close</Button>
          <Button
            disabled={style === ALL || applying}
            aria-describedby="{uid}-hint"
            onclick={() => void apply("project")}
          >
            Apply to project
          </Button>
          <Button
            variant="primary"
            disabled={style === ALL || applying}
            loading={applying}
            aria-describedby="{uid}-hint"
            onclick={() => void apply("layer")}
          >
            Apply to this layer only
          </Button>
        </div>
      {:else}
        <div class="field">
          <label for="{uid}-name">Name</label>
          <div class="input vf vf-within">
            <input
              bind:this={nameInput}
              id="{uid}-name"
              type="text"
              bind:value={name}
              maxlength={NAME_MAX}
              required
              autocomplete="off"
              spellcheck="false"
              disabled={choice === null || creating}
            />
          </div>
        </div>
        <div class="actions">
          <Button variant="ghost" onclick={cancel} disabled={creating}>Cancel</Button>
          <Button variant="primary" type="submit" disabled={!canCreate && !creating} loading={creating}>
            Create
          </Button>
        </div>
      {/if}
    </div>
  </form>
</dialog>

<style>
  .dialog {
    width: min(1040px, calc(100vw - 2 * var(--s-6)));
    max-height: calc(100vh - 2 * var(--s-6));
    margin: auto;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    color: var(--text);
    overflow: hidden;
  }
  /* Only the cards scroll: the title, chips and footer stay in view at any window height. */
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
    flex-direction: column;
    flex: 1 1 auto;
    min-height: 0;
    gap: var(--s-4);
    padding: var(--s-5);
  }
  .title {
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
    /* Room for the brackets of a focused chip. */
    padding: var(--vf-gap);
  }
  .chip {
    --vf-radius: var(--r-control);
    height: 28px;
    padding: 0 var(--s-3);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text-muted);
    font-weight: 500;
  }
  .chip:hover {
    color: var(--text);
    background: var(--surface-3);
  }
  /* Active filter: a quiet accent tint and edge, not a fill. */
  .chip[aria-pressed="true"] {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1px var(--accent);
    color: var(--text);
  }

  .body {
    display: flex;
    flex-direction: column;
    flex: 1 1 auto;
    min-height: 0;
    gap: var(--s-3);
    /* The scrollbar sits at the dialog's edge. */
    margin: 0 calc(-1 * var(--s-5));
    padding: 0 var(--s-5);
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  .body > .section-label {
    margin-top: var(--s-2);
  }
  .disclosure {
    --vf-radius: var(--r-control);
    display: inline-flex;
    align-items: center;
    align-self: flex-start;
    gap: var(--s-2);
    height: 28px;
    margin-top: var(--s-2);
    padding: 0 var(--s-2) 0 var(--s-1);
    border-radius: var(--r-control);
    color: var(--text-muted);
  }
  .disclosure:hover {
    color: var(--text);
  }
  .chev {
    display: inline-flex;
    transition: transform var(--dur) var(--ease);
  }
  .disclosure[aria-expanded="true"] .chev {
    transform: rotate(90deg);
  }
  .count {
    color: var(--text-muted);
    letter-spacing: 0;
  }

  .choices {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: var(--s-4);
    /* Room for the brackets, which sit --vf-gap outside each choice. */
    padding: var(--vf-gap);
  }
  .choices:focus-visible {
    outline: none;
  }
  .choice {
    --vf-radius: var(--r-panel);
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    min-width: 0;
    padding: var(--s-2) var(--s-2) var(--s-3);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
    cursor: pointer;
    user-select: none;
  }
  .choice:hover {
    background: var(--surface-3);
  }
  .choice[aria-selected="true"] {
    background: var(--selected-bg);
  }
  .thumb {
    position: relative;
    display: grid;
    place-items: center;
    aspect-ratio: 16 / 9;
    margin-bottom: var(--s-1);
    overflow: hidden;
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  .thumb img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .live {
    position: absolute;
    inset: 0;
    display: block;
  }
  .thumb-name {
    padding: 0 var(--s-2);
    color: var(--checker-dark-ink);
    font-family: var(--font-heading);
    font-size: var(--fs-sm);
    font-weight: 600;
    text-align: center;
  }
  .name {
    padding: 0 var(--s-1);
    font-weight: 600;
  }
  .desc {
    padding: 0 var(--s-1);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .styles {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-1);
    margin-top: var(--s-1);
    padding: 0 var(--s-1);
  }
  .style {
    padding: 0 6px;
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text-muted);
    font-size: var(--fs-xs);
    line-height: 18px;
  }
  .style.on {
    box-shadow: inset 0 0 0 1px var(--accent);
    color: var(--text);
  }

  .foot {
    display: flex;
    flex: none;
    flex-wrap: wrap;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--s-4);
    padding-top: var(--s-4);
    border-top: 1px solid var(--line);
  }
  .hint {
    align-self: center;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .field {
    display: grid;
    flex: 1 1 280px;
    max-width: 420px;
    gap: var(--s-1);
  }
  label {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .input {
    --vf-radius: var(--r-control);
    display: flex;
    height: var(--control-h);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  input {
    flex: 1;
    min-width: 0;
    padding: 0 var(--s-3);
    border: 0;
    background: transparent;
  }
  input:disabled {
    opacity: 0.5;
  }
  .actions {
    display: flex;
    gap: var(--s-2);
  }

  @keyframes enter {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
</style>

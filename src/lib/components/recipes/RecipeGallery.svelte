<!--
  RecipeGallery: "Add recipe" in /flows, "Add automation…" in Control (lazy chunk). A native modal <dialog>: one card per built-in
  recipe (the static thumbnail of its first preset template, name, description, its panel kind),
  then the install form: a name (the recipe's by default) and where its layers go: an existing
  screen (the one selected in Control by default) or a new one (name + resolution). Install
  creates everything (`installRecipe`), closes the dialog and hands the instance to the parent.
  A failed install shows the error inline and keeps the dialog open.

  Keyboard: the cards are a listbox (arrows move, Enter/Space choose, double-click installs);
  focus goes to the first card on open and back to the opener on close. Thumbnails are static
  images loaded lazily, never live iframes.

  Props: GalleryProps (./lazy.ts).
-->
<script lang="ts">
  import { tick } from "svelte";
  import { toAppError } from "$lib/api";
  import { installRecipe, listRecipes } from "$lib/api/recipes";
  import Button from "$lib/components/Button.svelte";
  import Select from "$lib/components/Select.svelte";
  import Segmented from "$lib/components/control/Segmented.svelte";
  import { RESOLUTION_PRESETS, defaultScreenName, validScreenName } from "$lib/components/screens/new/format";
  import { thumbUrl } from "$lib/components/templates/gallery/thumbs";
  import { SCREEN_NAME_MAX, type RecipeInfo, type RecipeScreen } from "$lib/types";
  import { PANEL_LABEL, type GalleryProps } from "./lazy";

  let { open, onclose, screens, selectedScreenId, oninstalled, title = "Add recipe" }: GalleryProps = $props();

  const NAME_MAX = 100;
  const uid = $props.id();
  let dialog: HTMLDialogElement | undefined = $state();
  let grid: HTMLDivElement | undefined = $state();
  let recipes = $state.raw<RecipeInfo[]>([]);
  let loadError = $state<string | null>(null);
  let thumbs = $state.raw<Record<string, string | null>>({});
  let chosen = $state<string | null>(null);
  let active = $state(0);
  let name = $state("");
  let mode = $state<"existing" | "new">("existing");
  let screenId = $state("");
  let screenName = $state("");
  let resolution = $state<string>(RESOLUTION_PRESETS[0].key);
  let installing = $state(false);
  let error = $state<string | null>(null);
  let returnFocus: HTMLElement | null = null;

  const choice = $derived(recipes.find((r) => r.id === chosen) ?? null);
  const tabStop = $derived(chosen !== null ? recipes.findIndex((r) => r.id === chosen) : active);
  const screenOptions = $derived(screens.map((v) => ({ value: v.screen.id, label: v.screen.name })));
  const MODES = [
    { value: "existing", label: "Existing screen" },
    { value: "new", label: "New screen" },
  ] as const;
  const RES_OPTIONS = RESOLUTION_PRESETS.map((p) => ({ value: p.key as string, label: `${p.label} · ${p.width}×${p.height}` }));
  const validName = $derived(name.trim().length >= 1 && name.trim().length <= NAME_MAX);
  const screenOk = $derived(
    mode === "existing" ? screens.some((v) => v.screen.id === screenId) : validScreenName(screenName),
  );
  const canInstall = $derived(choice !== null && validName && screenOk && !installing);

  $effect(() => {
    let alive = true;
    listRecipes().then(
      (list) => {
        if (!alive) return;
        recipes = list;
        for (const r of list) {
          const first = r.templates[0];
          if (!first) continue;
          void thumbUrl(first).then((url) => {
            if (alive) thumbs = { ...thumbs, [r.id]: url };
          });
        }
      },
      (e: unknown) => {
        if (alive) loadError = toAppError(e).message;
      },
    );
    return () => {
      alive = false;
    };
  });

  $effect(() => {
    const d = dialog;
    if (!d) return;
    if (open && !d.open) {
      returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      reset();
      if (typeof d.showModal === "function") d.showModal();
      else d.setAttribute("open", "");
      void tick().then(() => grid?.querySelector<HTMLElement>('[role="option"]')?.focus());
    } else if (!open && d.open) {
      shut(d);
    }
  });

  $effect(() => () => {
    if (dialog?.open) shut(dialog);
  });

  function reset() {
    chosen = null;
    active = 0;
    name = "";
    error = null;
    installing = false;
    screenId = selectedScreenId && screens.some((v) => v.screen.id === selectedScreenId) ? selectedScreenId : (screens[0]?.screen.id ?? "");
    mode = screenId ? "existing" : "new";
    screenName = defaultScreenName(screens.map((v) => v.screen.name));
    resolution = RESOLUTION_PRESETS[0].key;
  }

  function shut(d: HTMLDialogElement) {
    if (typeof d.close === "function") d.close();
    else d.removeAttribute("open");
    const el = returnFocus;
    returnFocus = null;
    if (el?.isConnected) el.focus();
  }

  function cancel() {
    if (!installing) onclose();
  }

  function onCancelEvent(e: Event) {
    e.preventDefault();
    cancel();
  }

  function onBackdropClick(e: MouseEvent) {
    if (e.target === dialog) cancel();
  }

  function pick(r: RecipeInfo) {
    if (chosen === r.id) return;
    chosen = r.id;
    name = r.name;
    screenName = r.name.slice(0, SCREEN_NAME_MAX);
  }

  function columns(): number {
    const cols = grid ? getComputedStyle(grid).gridTemplateColumns.split(" ").filter(Boolean).length : 1;
    return Math.max(1, cols);
  }

  function onGridKeydown(e: KeyboardEvent) {
    const n = recipes.length;
    if (n === 0) return;
    const cols = columns();
    const delta: Record<string, number> = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: cols, ArrowUp: -cols };
    let i: number | null = null;
    if (e.key in delta) i = Math.max(0, Math.min(n - 1, tabStop + delta[e.key]));
    else if (e.key === "Home") i = 0;
    else if (e.key === "End") i = n - 1;
    if (i === null) return;
    e.preventDefault();
    if (chosen !== null) pick(recipes[i]);
    else active = i;
    const to = i;
    void tick().then(() => grid?.querySelectorAll<HTMLElement>('[role="option"]')[to]?.focus());
  }

  function onCardKeydown(e: KeyboardEvent, r: RecipeInfo) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      pick(r);
    }
  }

  async function install(e?: SubmitEvent) {
    e?.preventDefault();
    if (!canInstall || !choice) return;
    const res = RESOLUTION_PRESETS.find((p) => p.key === resolution) ?? RESOLUTION_PRESETS[0];
    const screen: RecipeScreen =
      mode === "existing"
        ? { kind: "existing", screenId }
        : { kind: "new", name: screenName.trim(), width: res.width, height: res.height };
    installing = true;
    error = null;
    try {
      const inst = await installRecipe({ recipeId: choice.id, name: name.trim(), screen });
      installing = false;
      onclose();
      oninstalled(inst);
    } catch (err) {
      error = toAppError(err).message;
      installing = false;
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog bind:this={dialog} class="dialog" aria-labelledby="{uid}-title" oncancel={onCancelEvent} onclick={onBackdropClick}>
  <form class="inner" onsubmit={install} novalidate>
    <h2 id="{uid}-title" class="title">{title}</h2>

    {#if loadError}
      <p class="error" role="alert">Couldn't load the recipes: {loadError}</p>
    {/if}
    <div class="choices" role="listbox" tabindex="-1" aria-label="Recipes" bind:this={grid} onkeydown={onGridKeydown}>
      {#each recipes as r, i (r.id)}
        {@const thumb = thumbs[r.id]}
        <div
          class="choice vf"
          role="option"
          aria-selected={chosen === r.id}
          tabindex={i === tabStop ? 0 : -1}
          data-recipe-choice={r.id}
          onclick={() => pick(r)}
          ondblclick={() => {
            pick(r);
            void install();
          }}
          onkeydown={(e) => onCardKeydown(e, r)}
        >
          <span class="thumb checker-dark">
            {#if thumb}
              <img src={thumb} alt="" loading="lazy" decoding="async" width="320" height="180" />
            {:else}
              <span class="thumb-name" aria-hidden="true">{r.name}</span>
            {/if}
          </span>
          <span class="name-row">
            <span class="name">{r.name}</span>
            <span class="kind">{PANEL_LABEL[r.panel]}</span>
          </span>
          <span class="desc">{r.description}</span>
          {#if r.templates.length > 1}
            <span class="meta">{r.templates.length} templates</span>
          {/if}
        </div>
      {/each}
    </div>

    <div class="foot">
      <div class="fields">
        <div class="field">
          <label for="{uid}-name">Name</label>
          <div class="input vf vf-within">
            <input
              id="{uid}-name"
              type="text"
              bind:value={name}
              maxlength={NAME_MAX}
              autocomplete="off"
              spellcheck="false"
              disabled={choice === null || installing}
            />
          </div>
        </div>
        <div class="field">
          <span class="lbl" id="{uid}-where">Layers go on</span>
          <Segmented label="Layers go on" options={MODES} value={mode} onchange={(m) => (mode = m)} />
        </div>
        {#if mode === "existing"}
          <div class="field">
            <label for="{uid}-screen">Screen</label>
            <Select
              id="{uid}-screen"
              value={screenId}
              options={screenOptions.length > 0 ? screenOptions : [{ value: "", label: "No screens yet", disabled: true }]}
              onchange={(v) => (screenId = v)}
              disabled={screenOptions.length === 0}
            />
          </div>
        {:else}
          <div class="field">
            <label for="{uid}-sname">Screen name</label>
            <div class="input vf vf-within">
              <input id="{uid}-sname" type="text" bind:value={screenName} maxlength={SCREEN_NAME_MAX} autocomplete="off" spellcheck="false" />
            </div>
          </div>
          <div class="field">
            <label for="{uid}-res">Resolution</label>
            <Select id="{uid}-res" value={resolution} options={RES_OPTIONS} onchange={(v) => (resolution = v)} />
          </div>
        {/if}
      </div>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      <div class="actions">
        <Button variant="ghost" onclick={cancel} disabled={installing}>Cancel</Button>
        <Button variant="primary" type="submit" disabled={!canInstall && !installing} loading={installing}>Install</Button>
      </div>
    </div>
  </form>
</dialog>

<style>
  .dialog {
    width: min(960px, calc(100vw - 2 * var(--s-6)));
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
    animation: enter var(--dur) var(--ease);
  }
  .dialog::backdrop {
    background: var(--overlay);
    animation: fade var(--dur) var(--ease);
  }
  .inner {
    display: flex;
    flex-direction: column;
    gap: var(--s-4);
    padding: var(--s-5);
  }
  .title {
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .choices {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: var(--s-4);
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
  .thumb-name {
    padding: 0 var(--s-2);
    color: var(--checker-dark-ink);
    font-family: var(--font-heading);
    font-size: var(--fs-sm);
    font-weight: 600;
    text-align: center;
  }
  .name-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--s-2);
    padding: 0 var(--s-1);
  }
  .name {
    font-weight: 600;
  }
  .kind,
  .meta {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .meta {
    padding: 0 var(--s-1);
  }
  .desc {
    padding: 0 var(--s-1);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .foot {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding-top: var(--s-4);
    border-top: 1px solid var(--line);
  }
  .fields {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: var(--s-3) var(--s-4);
    align-items: end;
  }
  .field {
    display: grid;
    gap: var(--s-1);
    min-width: 0;
  }
  label,
  .lbl {
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
    justify-content: flex-end;
    gap: var(--s-2);
  }
  .error {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-sm);
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
  @media (prefers-reduced-motion: reduce) {
    .dialog[open],
    .dialog::backdrop {
      animation: none;
    }
  }
</style>

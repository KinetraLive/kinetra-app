<!--
  VariantTabs: the template's looks as a tablist above the editor (name, thumbnail, a "Default"
  badge on the default one, a warning dot when the variant leaves fields unused), the active
  variant's ⋯ actions button and the "Not used in <variant>: …" line.

  Lazy parts: the actions menu and its dialogs (./VariantMenu.svelte, loaded on first open) and
  the thumbnails (./thumbs.ts with the renderer's compile, loaded after first paint). Thumbnails
  re-render 300 ms after a variant, the fields or the variables change (working copies change on
  save), one timer only while a render is pending; they are released on unmount.

  Props:
    variants: VariantDoc[]      working copies (html/css as last saved or switched away from)
    active: string              the variant being edited
    defaultVariant: string
    fields, variables           shared schema (warnings, thumbnail values)
    panelId: string             id of the tabpanel the tabs control
    onselect: (id) => void
    actions: VariantActions     duplicate / rename / delete / set default (see VariantMenu)
-->
<script lang="ts" module>
  export interface VariantActions {
    duplicate: (name: string) => Promise<boolean>;
    rename: (id: string, name: string) => Promise<boolean>;
    remove: (id: string) => Promise<boolean>;
    setDefault: (id: string) => Promise<boolean>;
  }
</script>

<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import Ellipsis from "lucide-svelte/icons/ellipsis";
  import type { FieldDef, VariableDef, VariantDoc } from "$lib/types";
  import { unusedFields } from "./variants";

  let {
    variants,
    active,
    defaultVariant,
    fields,
    variables,
    panelId,
    onselect,
    actions,
  }: {
    variants: readonly VariantDoc[];
    active: string;
    defaultVariant: string;
    fields: readonly FieldDef[];
    variables: readonly VariableDef[];
    panelId: string;
    onselect: (id: string) => void;
    actions: VariantActions;
  } = $props();

  const uid = $props.id();

  const unused = $derived(new Map(variants.map((v) => [v.id, unusedFields(v.html, fields)])));
  const current = $derived(variants.find((v) => v.id === active) ?? variants[0]);
  const currentUnused = $derived(current ? (unused.get(current.id) ?? []) : []);

  // ---- Keyboard (roving tab stop, automatic activation) ----

  let tabs: HTMLButtonElement[] = $state([]);

  function onkey(e: KeyboardEvent, i: number) {
    const n = variants.length;
    let next = -1;
    if (e.key === "ArrowRight") next = (i + 1) % n;
    else if (e.key === "ArrowLeft") next = (i - 1 + n) % n;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = n - 1;
    if (next < 0) return;
    e.preventDefault();
    onselect(variants[next].id);
    tabs[next]?.focus();
  }

  // ---- Actions menu (lazy) ----

  type MenuT = typeof import("./VariantMenu.svelte").default;
  let Menu = $state<MenuT | null>(null);
  let menuOpen = $state(false);
  let more: HTMLButtonElement | undefined = $state();

  async function openMenu() {
    if (menuOpen) {
      menuOpen = false;
      return;
    }
    Menu ??= (await import("./VariantMenu.svelte")).default;
    menuOpen = true;
  }

  function closeMenu(refocus: boolean) {
    menuOpen = false;
    if (refocus) void tick().then(() => more?.focus());
  }

  // ---- Thumbnails (lazy) ----

  type Thumbs = typeof import("./thumbs");
  let thumbs: Thumbs | null = null;
  const els = new Map<string, HTMLElement>();
  /** What each element last rendered (identity of its inputs). */
  const shown = new Map<HTMLElement, readonly unknown[]>();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let alive = true;

  function render() {
    timer = undefined;
    if (!alive || !thumbs) return;
    for (const v of variants) {
      const el = els.get(v.id);
      if (!el) continue;
      const key = [v, fields, variables];
      const last = shown.get(el);
      if (last && last.every((x, i) => x === key[i])) continue;
      try {
        thumbs.renderThumb(el, v, fields, variables);
        shown.set(el, key);
      } catch {
        // A template the renderer can't compile simply has no thumbnail.
      }
    }
  }

  function queue() {
    if (timer === undefined && thumbs) timer = setTimeout(render, 300);
  }

  function thumb(id: string) {
    return (el: HTMLElement) => {
      els.set(id, el);
      queue();
      return () => {
        if (els.get(id) !== el) return; // already released on unmount
        els.delete(id);
        shown.delete(el);
        thumbs?.releaseThumb(el);
      };
    };
  }

  $effect(() => {
    void [variants, fields, variables];
    queue();
  });

  onMount(() => {
    import("./thumbs").then(
      (m) => {
        if (!alive) return;
        thumbs = m;
        render();
      },
      () => {},
    );
    return () => {
      alive = false;
      if (timer !== undefined) clearTimeout(timer);
      timer = undefined;
      for (const el of els.values()) thumbs?.releaseThumb(el);
      els.clear();
      shown.clear();
    };
  });
</script>

<div class="variants">
  <div class="row">
    <div class="tabs" role="tablist" aria-label="Variants">
      {#each variants as v, i (v.id)}
        {@const warn = (unused.get(v.id)?.length ?? 0) > 0}
        <button
          bind:this={tabs[i]}
          type="button"
          role="tab"
          class="tab"
          id="{uid}-v-{v.id}"
          data-variant={v.id}
          aria-selected={v.id === active}
          aria-controls={panelId}
          tabindex={v.id === active ? 0 : -1}
          onclick={() => onselect(v.id)}
          onkeydown={(e) => onkey(e, i)}
        >
          <span class="thumb" aria-hidden="true" {@attach thumb(untrack(() => v.id))}></span>
          <span class="name">{v.name}</span>
          {#if v.id === defaultVariant}<span class="badge">Default</span>{/if}
          {#if warn}<span class="dot" role="img" aria-label="Has unused fields"></span>{/if}
        </button>
      {/each}
    </div>
    <span class="more-wrap">
      <button
        bind:this={more}
        type="button"
        class="more vf"
        aria-label="Variant actions for {current?.name ?? ''}"
        title="Variant actions"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        onclick={() => void openMenu()}
        onkeydown={(e) => {
          if (e.key === "ArrowDown" && !menuOpen) {
            e.preventDefault();
            void openMenu();
          }
        }}
      >
        <Ellipsis size={16} />
      </button>
      {#if Menu && menuOpen && current}
        <Menu
          variant={current}
          isDefault={current.id === defaultVariant}
          names={variants.map((v) => v.name)}
          count={variants.length}
          {actions}
          onclose={closeMenu}
        />
      {/if}
    </span>
  </div>
  {#if current && currentUnused.length > 0}
    <p class="unused" role="status">
      Not used in {current.name}: <span class="mono">{currentUnused.map((f) => f.key).join(", ")}</span>
    </p>
  {/if}
</div>

<style>
  .variants {
    border-bottom: 1px solid var(--line);
    background: var(--surface-1);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    padding: var(--s-1) var(--s-2);
  }
  .tabs {
    display: flex;
    gap: var(--s-1);
    min-width: 0;
    overflow-x: auto;
  }
  .tab {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: var(--s-2);
    height: var(--row-h);
    padding: 0 var(--s-2) 0 var(--s-1);
    border-radius: var(--r-control);
    color: var(--text-muted);
    font-weight: 500;
  }
  .tab:hover {
    color: var(--text);
    background: var(--surface-2);
  }
  .tab:focus-visible {
    outline-offset: -2px;
  }
  /* Selected: the same look as a selected .vf card (subtle surface + 1px accent border). */
  .tab[aria-selected="true"] {
    color: var(--text);
    background: var(--selected-bg);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .thumb {
    display: block;
    flex: none;
    width: 60px;
    height: 26px;
    border-radius: 3px;
    background: var(--checker-dark-1);
    box-shadow: inset 0 0 0 1px var(--line);
    overflow: hidden;
  }
  .name {
    max-width: 12rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .badge {
    padding: 0 var(--s-1);
    border-radius: 4px;
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text-muted);
    font-size: var(--fs-xs);
    line-height: 16px;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: var(--r-full);
    background: var(--warning);
  }
  .more-wrap {
    position: relative;
    flex: none;
  }
  .more {
    --vf-radius: var(--r-control);
    display: inline-grid;
    place-items: center;
    width: var(--row-h);
    height: var(--row-h);
    border-radius: var(--r-control);
    color: var(--text-muted);
  }
  .more:hover,
  .more[aria-expanded="true"] {
    color: var(--text);
    background: var(--surface-2);
  }
  .unused {
    padding: 0 var(--s-3) var(--s-2);
    color: var(--warning);
    font-size: var(--fs-sm);
  }
</style>

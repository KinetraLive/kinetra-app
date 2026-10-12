<!--
  Automation: the AUTOMATION section of Control's side panel for the selected screen (lazy chunk; a
  LazyStrip loads it). The model is ./automation.ts (scoping, broken detection, summaries,
  aggregated warnings); operator words only: automations (installed recipes) and rules (flows).

    header     "Add automation…" (the gallery, a lazy chunk)
    attention  "N automations need attention": one line per broken automation (source, layer or
               screen gone; never rendered in full) with Fix… (FixRecipe, lazy: pick a new source,
               or explain and offer Delete) and Delete… (RemoveRecipe, lazy: keep / delete created)
    rows       one collapsed row per automation of this screen: name (a qualifier when names
               clash), "N rules · N armed · N warnings", ⋯ (Show its screen, Edit in Flows,
               Remove…). Expanded: its live operator panel (schedule, countdown or slideshow; each a
               lazy chunk loaded when a row of that kind is first expanded), its warnings (each
               message once, with the rules it affects), its rules (FlowList) and New rule
               (owned by it, aimed at its screen and source)
    other      "Other rules": the screen's own rules (not of any automation, not cues), same shape
    empty      "Nothing runs on its own here." with Add automation… and New rule

  Collapsed rows unmount (no clock, no listeners); which rows are open is kept for the session.

  Props: AutomationProps (./lazy.ts)
-->
<script lang="ts">
  import type { Component } from "svelte";
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import Ellipsis from "lucide-svelte/icons/ellipsis";
  import Plus from "lucide-svelte/icons/plus";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import { goto } from "$app/navigation";
  import { toAppError } from "$lib/api";
  import Button from "$lib/components/Button.svelte";
  import type { MenuItem } from "$lib/components/control/Menu.svelte";
  import FlowList from "$lib/components/flows/FlowList.svelte";
  import type { Lookups } from "$lib/components/flows/linear";
  import { live } from "$lib/stores/live.svelte";
  import { isAdvanced } from "$lib/stores/mode.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { FlowInfo, RecipeInstance } from "$lib/types";
  import { automationOf, summaryText, type Broken, type Qualifier, type Warning } from "./automation";
  import {
    loadFix,
    loadGallery,
    loadRemove,
    loadSchedulePanel,
    loadSlideshowPanel,
    type AutomationProps,
    type FixProps,
    type GalleryProps,
    type RemoveProps,
    type SchedulePanelProps,
    type SlideshowPanelProps,
  } from "./lazy";
  import { OTHER_ROW, RecipesState } from "./recipes.svelte";

  let { ctl, Menu }: AutomationProps = $props();

  const uid = $props.id();
  const recipes = new RecipesState();
  $effect(() => recipes.connect());

  const sources = $derived(new Map(ctl.sources.map((s) => [s.id, s])));
  const view = $derived(
    automationOf({
      screenId: ctl.selectedId ?? "",
      recipes: recipes.list,
      flows: ctl.flows.list,
      sources,
      screens: live.screens,
      ready: recipes.loaded && ctl.sourcesLoaded && live.loaded,
    }),
  );
  const empty = $derived(
    recipes.loaded && ctl.flows.loaded && view.broken.length === 0 && view.recipes.length === 0 && !view.other,
  );

  const lookups = $derived<Lookups>({
    screens: new Map(ctl.screens.map((v) => [v.screen.id, v])),
    sources,
    flows: new Map(ctl.flows.list.map((f) => [f.id, f])),
  });

  const screenName = (id: string) => live.screens.get(id)?.screen.name ?? "";

  // ---- Panels (lazy per kind) ----------------------------------------------------------------------

  let SchedulePanel = $state.raw<Component<SchedulePanelProps> | null>(null);
  let SlideshowPanel = $state.raw<Component<SlideshowPanelProps> | null>(null);

  $effect(() => {
    for (const { recipe: r } of view.recipes) {
      if (!recipes.expanded.has(r.id) || !r.sourceId) continue;
      if (r.panel === "slideshow") {
        if (!SlideshowPanel) loadSlideshowPanel().then((c) => (SlideshowPanel = c), () => {});
      } else if (!SchedulePanel) {
        loadSchedulePanel().then((c) => (SchedulePanel = c), () => {});
      }
    }
  });

  // ---- Dialogs (lazy) -------------------------------------------------------------------------------

  let Gallery = $state.raw<Component<GalleryProps> | null>(null);
  let galleryOpen = $state(false);
  let Remove = $state.raw<Component<RemoveProps> | null>(null);
  let removing = $state<string | null>(null);
  let Fix = $state.raw<Component<FixProps> | null>(null);
  let fixing = $state<string | null>(null);

  const removeTarget = $derived(removing ? recipes.get(removing) : undefined);
  const fixTarget = $derived(fixing ? view.broken.find((b) => b.recipe.id === fixing) : undefined);

  function failed(e: unknown) {
    toasts.push({ kind: "error", message: `Couldn't open it: ${toAppError(e).message}` });
  }

  async function openGallery() {
    try {
      Gallery ??= await loadGallery();
      galleryOpen = true;
    } catch (e) {
      failed(e);
    }
  }

  async function openRemove(id: string) {
    try {
      Remove ??= await loadRemove();
      fixing = null;
      removing = id;
    } catch (e) {
      failed(e);
    }
  }

  async function openFix(id: string) {
    try {
      Fix ??= await loadFix();
      fixing = id;
    } catch (e) {
      failed(e);
    }
  }

  function installed(inst: RecipeInstance) {
    recipes.put(inst);
    recipes.expand(inst.id);
    // It created a source, layers, rules and maybe a screen: refresh what Control shows.
    void live.refresh();
    void ctl.loadSources();
    void ctl.flows.load();
    if (ctl.visible.some((v) => v.screen.id === inst.screenId)) ctl.select(inst.screenId);
    const screen = screenName(inst.screenId);
    toasts.push({ kind: "success", message: `${inst.name} is ready${screen ? ` on ${screen}` : ""}.` });
  }

  function removed(inst: RecipeInstance, deleteCreated: boolean) {
    recipes.drop(inst.id);
    removing = null;
    // Its rules go with it.
    void ctl.flows.load();
    if (deleteCreated) {
      void live.refresh();
      void ctl.loadSources();
    }
    toasts.push({ kind: "success", message: `Removed ${inst.name}.` });
  }

  function fixed(inst: RecipeInstance) {
    recipes.put(inst);
    fixing = null;
    // Its rules point at the new source now (and their warnings changed).
    void ctl.flows.load();
    void live.refresh();
    const source = inst.sourceId ? sources.get(inst.sourceId)?.name : undefined;
    toasts.push({ kind: "success", message: `${inst.name} runs on ${source ?? "its new source"} now.` });
  }

  /** A rule owned by the automation (listed under it, removed with it), aimed at its screen and source. */
  function newRule(r: RecipeInstance | null) {
    void ctl.flowEditor.show(
      r
        ? {
            mode: "new",
            kind: "rule",
            origin: { id: r.id, screenId: r.screenId, layerId: r.layerIds[0] ?? "", sourceId: r.sourceId },
          }
        : { mode: "new", kind: "rule" },
    );
  }

  function menuItems(r: RecipeInstance): MenuItem[] {
    const items: MenuItem[] = [];
    if (r.screenId !== ctl.selectedId) {
      items.push({
        id: "screen",
        label: "Show its screen",
        meta: screenName(r.screenId),
        disabled: !ctl.visible.some((v) => v.screen.id === r.screenId),
        onselect: () => ctl.select(r.screenId),
      });
    }
    // Simple mode hides the flow editor.
    if (isAdvanced()) items.push({ id: "flows", label: "Edit in Flows", onselect: () => void goto("/flows") });
    items.push(
      { id: "remove", label: "Remove…", danger: true, onselect: () => void openRemove(r.id) },
    );
    return items;
  }

  /** "Its source and layer were deleted." */
  function reason(b: Broken): string {
    const gone: string[] = [];
    if (b.sourceMissing) gone.push("source");
    if (b.screenMissing) gone.push("screen");
    else if (b.missingLayers.length > 0) gone.push(b.missingLayers.length === 1 ? "layer" : "layers");
    const many = gone.length > 1 || gone[0] === "layers";
    return `Its ${gone.join(" and ")} ${many ? "were" : "was"} deleted.`;
  }

  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
</script>

{#snippet title(name: string, q: Qualifier | null)}
  <span class="name">{name}</span>
  {#if q}<span class="q" class:mono={q.mono}>— {q.text}</span>{/if}
{/snippet}

{#snippet body(id: string, owner: RecipeInstance | null, rules: FlowInfo[], warnings: Warning[], label: string)}
  <div class="body" id="{uid}-{id}">
    {#if owner?.sourceId}
      {#if owner.panel === "slideshow"}
        {#if SlideshowPanel}<SlideshowPanel {ctl} sourceId={owner.sourceId} />{/if}
      {:else if SchedulePanel}
        <SchedulePanel
          sourceId={owner.sourceId}
          kind={owner.panel}
          name={owner.name}
          updatedAt={sources.get(owner.sourceId)?.updatedAt ?? null}
          {Menu}
        />
      {/if}
    {/if}
    <div class="rules">
      {#if warnings.length > 0}
        <ul class="warnings" aria-label="Warnings of {label}">
          {#each warnings as w (w.message)}
            <li class="warning">
              <span class="icon" aria-hidden="true"><TriangleAlert size={12} /></span>
              <span class="wtext">
                <span>{w.message}</span>
                <span class="affects">Affects {w.rules.join(", ")}</span>
              </span>
            </li>
          {/each}
        </ul>
      {/if}
      <div class="fhead">
        <span class="section-label">Rules</span>
        <Button size="sm" variant="ghost" aria-label="New rule for {label}" onclick={() => newRule(owner)}>
          {#snippet icon()}<Plus size={14} />{/snippet}
          New rule
        </Button>
      </div>
      {#if rules.length > 0}
        <FlowList flows={rules} {ctl} {Menu} {lookups} label="Rules of {label}" />
      {:else}
        <p class="muted">No rules yet.</p>
      {/if}
    </div>
  </div>
{/snippet}

{#snippet row(
  id: string,
  owner: RecipeInstance | null,
  name: string,
  q: Qualifier | null,
  rules: FlowInfo[],
  armed: number,
  warnings: Warning[],
)}
  {@const open = recipes.expanded.has(id)}
  {@const label = q ? `${name} — ${q.text}` : name}
  <li class="row" class:other={!owner} data-recipe={owner?.id} data-row={id}>
    <div class="rhead">
      <button
        type="button"
        class="toggle vf"
        aria-expanded={open}
        aria-controls="{uid}-{id}"
        onclick={() => recipes.toggle(id)}
      >
        <span class="chev" class:open aria-hidden="true"><ChevronRight size={14} /></span>
        <span class="line">
          {@render title(name, q)}
          <span class="sum">· {summaryText(rules.length, armed, 0)}</span>
          {#if warnings.length > 0}
            <span class="sum warn">· {plural(warnings.length, "warning", "warnings")}</span>
          {/if}
        </span>
      </button>
      {#if owner}
        <Menu label="Actions for {label}" items={menuItems(owner)} align="end">
          {#snippet icon()}<Ellipsis size={14} />{/snippet}
        </Menu>
      {/if}
    </div>
    {#if open}{@render body(id, owner, rules, warnings, label)}{/if}
  </li>
{/snippet}

<header class="head">
  <h2 class="section-label" id="{uid}-title">Automation</h2>
  <Button size="sm" variant="ghost" onclick={openGallery}>
    {#snippet icon()}<Plus size={14} />{/snippet}
    Add automation…
  </Button>
</header>

{#if view.broken.length > 0}
  <div class="attn" role="group" aria-labelledby="{uid}-attn">
    <p class="attn-head" id="{uid}-attn">
      <span class="icon" aria-hidden="true"><TriangleAlert size={14} /></span>
      {view.broken.length}
      {view.broken.length === 1 ? "automation needs" : "automations need"} attention
    </p>
    <ul class="blist">
      {#each view.broken as b (b.recipe.id)}
        {@const label = b.qualifier ? `${b.recipe.name} — ${b.qualifier.text}` : b.recipe.name}
        <li class="broken" data-broken={b.recipe.id}>
          <span class="btext">
            <span class="line">{@render title(b.recipe.name, b.qualifier)}</span>
            <span class="why">{reason(b)}</span>
          </span>
          <Button size="sm" onclick={() => void openFix(b.recipe.id)} aria-label="Fix {label}">Fix…</Button>
          <Button size="sm" variant="danger" onclick={() => void openRemove(b.recipe.id)} aria-label="Delete {label}">
            Delete…
          </Button>
        </li>
      {/each}
    </ul>
  </div>
{/if}

{#if empty}
  <div class="empty">
    <p>Nothing runs on its own here.</p>
    <span class="acts">
      <Button size="sm" variant="primary" onclick={openGallery}>Add automation…</Button>
      <Button size="sm" variant="ghost" onclick={() => newRule(null)}>New rule</Button>
    </span>
  </div>
{:else if view.recipes.length > 0 || view.other}
  <ul class="list" aria-labelledby="{uid}-title">
    {#each view.recipes as r (r.recipe.id)}
      {@render row(r.recipe.id, r.recipe, r.recipe.name, r.qualifier, r.rules, r.armed, r.warnings)}
    {/each}
    {#if view.other}
      {@render row(OTHER_ROW, null, "Other rules", null, view.other.rules, view.other.armed, view.other.warnings)}
    {/if}
  </ul>
{/if}

{#if Gallery}
  <Gallery
    open={galleryOpen}
    onclose={() => (galleryOpen = false)}
    screens={ctl.screens}
    selectedScreenId={ctl.selectedId}
    oninstalled={installed}
    title="Add automation"
  />
{/if}

{#if Fix && fixTarget}
  {@const target = fixTarget}
  <Fix
    broken={target}
    sources={ctl.sources}
    screenName={screenName(target.recipe.screenId)}
    onclose={() => (fixing = null)}
    onfixed={fixed}
    ondelete={() => void openRemove(target.recipe.id)}
  />
{/if}

{#if Remove && removeTarget}
  {@const target = removeTarget}
  <Remove recipe={target} onclose={() => (removing = null)} onremoved={(del) => removed(target, del)} />
{/if}

<style>
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: calc(var(--row-h) + var(--s-2));
    padding: var(--s-1) var(--s-2) var(--s-1) var(--s-3);
    border-bottom: 1px solid var(--line);
  }
  .head h2 {
    margin: 0;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-2);
    padding: var(--s-3);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .empty p {
    margin: 0;
  }
  .acts {
    display: flex;
    gap: var(--s-1);
  }
  .attn {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    margin: var(--s-2);
    padding: var(--s-2);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--warning);
  }
  .attn-head {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    margin: 0;
    padding: 0 var(--s-1);
    color: var(--warning);
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  .blist,
  .list,
  .warnings {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .broken {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    min-height: var(--row-h);
    padding: var(--s-1);
  }
  .btext {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    line-height: 1.3;
  }
  .why {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .row + .row {
    border-top: 1px solid var(--line);
  }
  .rhead {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    padding: var(--s-1) var(--s-2);
  }
  .toggle {
    --vf-radius: var(--r-control);
    display: flex;
    flex: 1;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    height: var(--control-h);
    padding: 0 var(--s-1);
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    color: var(--text);
    text-align: left;
  }
  .toggle:hover {
    background: var(--surface-2);
  }
  .chev {
    display: inline-flex;
    flex: none;
    color: var(--text-muted);
    transition: transform var(--dur-fast) var(--ease);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .line {
    display: flex;
    align-items: baseline;
    gap: var(--s-1);
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }
  .line > * {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .name {
    flex: none;
    max-width: 60%;
    font-weight: 600;
  }
  /* "Other rules" is short: it always shows whole; the summary gives way instead. */
  .other .name {
    max-width: none;
  }
  .q,
  .sum {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .sum {
    flex: 0 1 auto;
    min-width: 0;
  }
  .sum.warn {
    color: var(--warning);
  }
  .rules {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    padding: var(--s-2) var(--s-2) var(--s-3);
  }
  .warning {
    display: flex;
    gap: var(--s-2);
    padding: var(--s-1);
    font-size: var(--fs-xs);
    line-height: 1.35;
  }
  .icon {
    display: inline-flex;
    flex: none;
    padding-top: 1px;
    color: var(--warning);
  }
  .wtext {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .affects,
  .muted {
    color: var(--text-muted);
  }
  .muted {
    margin: 0;
    font-size: var(--fs-sm);
  }
  .fhead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-left: var(--s-1);
  }
  @media (prefers-reduced-motion: reduce) {
    .chev {
      transition: none;
    }
  }
</style>

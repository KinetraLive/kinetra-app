<!--
  Inspector: the screen editor's right column for the selected layer. Name; template (switching
  keeps matching field values and variable overrides); the template's variant when it has more
  than one ("(default)" marked; a missing one says the default shows); the box as mono x / y / w / h plus "Fill
  screen", an aspect lock (W / H keep the ratio; the canvas resize too; no side below
  LAYER_SIZE_MIN; `aspectLocked` is the effective lock: on by default for layers that scale),
  align-to-screen buttons and "Content sizing" (Scale to fit / Reflow; absent = Reflow until the
  layer is resized). A typed W or H is a resize: `onrect(rect, true)` (the editor makes a layer
  that isn't explicitly Reflow Scale to fit), keeping the graphic's aspect (the design box's) while
  locked. A Scale to fit layer shows its scale ("Scale 64 %": the graphic's size over its design
  size) with "100 %" (the design size around the box's centre); field values per the template's fields; variable overrides through the shared
  VariableValuesForm ("Overrides template" + Reset per overridden key).

  Source bindings: each field has "Bind to source" (BindMenu: a source, then a column). A bound
  field shows the source's current value read-only, labelled "Source · Column", with Unbind. The
  layer's `sourceMissing` warnings show under the fields.

  List fields bind a whole source (ListBind, a lazy chunk: source, sort column, as numbers,
  descending, row limit). A list is one summary line that opens the spreadsheet drawer: "6 items ·
  Edit" (its own rows, edited through `onfield`) or, bound, "Linked to: Guests · Edit source" (the
  data table, saved to the source; `onsourcesaved` after each save), with binding options and
  Unbind. A schedule source binds a single field to its NOW or UP NEXT item
  ("Schedule · Up next · Title"; a list to all items). Clock fields have no binding; a timer field
  binds only to a schedule's countdown ("Schedule · Up next · Countdown"). Unbound timer fields send
  their actions through `ontimer` (the editor calls setTimer).

  Presentational: the editor owns the document; every change is a callback.

  Props:
    layer: ScreenLayer
    template: TemplateDoc | null     the layer's template (null while loading or missing)
    missing: boolean                 the layer's template doesn't exist
    templates: TemplateSummary[]     choices for the template switch
    screen: { width, height }
    onname: (name: string) => void
    ontemplate: (templateId: string) => void
    onvariant?: (variant: string) => void   another variant of the template (the editor sends it)
    variantMissing?: boolean          the layer's variant no longer exists (the default shows)
    onstyles?: () => void            "Browse styles…" under the Variant select (the style gallery for this layer)
    onrect: (rect: Rect, resized?: boolean) => void   resized: W or H typed
    onsizing?: (sizing: ContentSizing) => void   content sizing changed
    onopacity?: (opacity: number) => void   the layer's opacity, 0–100 (the Opacity slider; every move)
    onfit?: (rect: Rect) => void     "Fit box to graphic" (shown when the template has a design):
                                     this box (fitBox) and Scale to fit, as one edit. A template
                                     whose preset ships a newer look shows UpdateNotice (lazy):
                                     "This graphic doesn't follow its box. Update to the latest
                                     look" without a design, "A newer look is available · Update"
                                     with one
    aspectLocked?: boolean           W / H keep the ratio (and so does resizing on the canvas)
    onlockaspect?: () => void
    onalign?: (kind: AlignKind) => void   align to the screen
    onanchor?: (patch: Partial<LayerAnchor>) => void   the position anchor (AnchorPad: 3×3 grid,
                                     centre buttons, "Respect safe area")
    onfield: (key: string, value: string) => void
    onvar: (key: string, value: string) => void
    onresetvar: (key: string) => void
    sources?: SourceSummary[] | null   for bindings; null = not loaded yet
    sourceWarnings?: string[]          this layer's sourceMissing messages
    onloadsources?: () => void         a Bind menu opened (load the sources lazily)
    onbind?: (key: string, binding: SourceBinding) => void
    onunbind?: (key: string) => void
    ontimer?: (key: string, action: TimerAction) => void
    onsourcesaved?: () => void       a data table was saved from the list drawer (reload sources)
-->
<script module lang="ts">
  // The Bind menu is its own chunk: requested when the first Inspector mounts, so the editor
  // route's initial JS stays small (it appears a moment after the fields).
  const bindMenu = import("./BindMenu.svelte");
  // A media layer's sections (file, play range, sound, end): loaded only when one is selected.
  let mediaChunk: Promise<typeof import("$lib/components/media/MediaInspector.svelte")> | null = null;
  const loadMedia = () => (mediaChunk ??= import("$lib/components/media/MediaInspector.svelte"));
  // List binding options: loaded only when a list field shows.
  let listBindChunk: Promise<typeof import("./ListBind.svelte")> | null = null;
  const loadListBind = () => (listBindChunk ??= import("./ListBind.svelte"));
</script>

<script lang="ts">
  import { tick } from "svelte";
  import Lock from "lucide-svelte/icons/lock";
  import LockOpen from "lucide-svelte/icons/lock-open";
  import Maximize from "lucide-svelte/icons/maximize";
  import Scan from "lucide-svelte/icons/scan";
  import Unlink from "lucide-svelte/icons/unlink";
  import IconButton from "$lib/components/IconButton.svelte";
  import Button from "$lib/components/Button.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import OpacityInput from "$lib/components/inputs/OpacityInput.svelte";
  import Select from "$lib/components/Select.svelte";
  import FieldInputs from "$lib/components/fields/FieldInputs.svelte";
  import type { LinkedList } from "$lib/components/fields/drawer";
  import VariableValuesForm from "$lib/components/variables/VariableValuesForm.svelte";
  import type {
    ContentSizing,
    LayerAnchor,
    LiveLayerView,
    Rect,
    ScreenLayer,
    SourceBinding,
    SourceSummary,
    TemplateDoc,
    TemplateSummary,
    TimerAction,
  } from "$lib/types";
  import AlignBar from "./AlignBar.svelte";
  import AnchorPad from "./AnchorPad.svelte";
  import type { AlignKind } from "./align";
  import { variantOptions } from "../variants.svelte";
  import {
    LAYER_NAME_MAX,
    LAYER_SIZE_MIN,
    bindingText,
    clampRect,
    effectiveVars,
    fitBox,
    fullRect,
    overrideErrors,
    actualSize,
    fitScale,
    graphicRatio,
    sameRect,
    scaleText,
    scalesOnResize,
    sizeRect,
  } from "./shared";

  let {
    layer,
    template,
    missing,
    templates,
    screen,
    onname,
    ontemplate,
    onvariant = () => {},
    variantMissing = false,
    onstyles,
    onrect,
    onsizing = () => {},
    onopacity = () => {},
    onfit = () => {},
    aspectLocked = false,
    onlockaspect = () => {},
    onalign = () => {},
    onanchor = () => {},
    onfield,
    onvar,
    onresetvar,
    sources = null,
    sourceWarnings = [],
    onloadsources = () => {},
    onbind = () => {},
    onunbind = () => {},
    ontimer,
    onsourcesaved = () => {},
    screenId = "",
    live = null,
    onpatch = () => {},
  }: {
    layer: ScreenLayer;
    template: TemplateDoc | null;
    missing: boolean;
    templates: readonly TemplateSummary[];
    screen: { width: number; height: number };
    onname: (name: string) => void;
    ontemplate: (templateId: string) => void;
    onvariant?: (variant: string) => void;
    variantMissing?: boolean;
    onstyles?: () => void;
    onrect: (rect: Rect, resized?: boolean) => void;
    onsizing?: (sizing: ContentSizing) => void;
    onopacity?: (opacity: number) => void;
    onfit?: (rect: Rect) => void;
    aspectLocked?: boolean;
    onlockaspect?: () => void;
    onalign?: (kind: AlignKind) => void;
    onanchor?: (patch: Partial<LayerAnchor>) => void;
    onfield: (key: string, value: string) => void;
    onvar: (key: string, value: string) => void;
    onresetvar: (key: string) => void;
    sources?: readonly SourceSummary[] | null;
    sourceWarnings?: readonly string[];
    onloadsources?: () => void;
    onbind?: (key: string, binding: SourceBinding) => void;
    onunbind?: (key: string) => void;
    ontimer?: (key: string, action: TimerAction) => void;
    onsourcesaved?: () => void;
    /** Media layers: the screen (transport), the layer's live view (playback, readiness) and a
     * patch of the layer (its `media` config). */
    screenId?: string;
    live?: LiveLayerView | null;
    onpatch?: (patch: Partial<ScreenLayer>) => void;
  } = $props();

  /** The layer when it shows a template (media layers have their own sections). */
  const tl = $derived(layer.kind !== "media" ? layer : null);

  let root: HTMLDivElement | undefined = $state();

  /** After Bind / Unbind the control swaps: keep keyboard focus on the field's new control. */
  async function refocus(key: string) {
    await tick();
    root?.querySelector<HTMLElement>(`[data-bindctl="${CSS.escape(key)}"] button`)?.focus();
  }

  const sourceById = $derived(new Map((sources ?? []).map((s) => [s.id, s])));
  /** Bound field key → "Source · Column". */
  const boundLabels = $derived.by(() => {
    const out: Record<string, string> = {};
    for (const [key, b] of Object.entries(tl?.bindings ?? {})) {
      out[key] = sources === null ? "Source" : bindingText(sourceById.get(b.sourceId) ?? null, b);
    }
    return out;
  });
  /** Bound list fields → the source they are linked to (the summary's "Linked to: …"). */
  const linked = $derived.by(() => {
    const out: Record<string, LinkedList> = {};
    for (const [key, b] of Object.entries(tl?.bindings ?? {})) {
      const s = b.column === "" ? sourceById.get(b.sourceId) : undefined;
      if (!s) continue;
      out[key] = { sourceId: s.id, name: s.name, kind: s.kind, usedBy: s.usedBy, detail: boundLabels[key], onsaved: onsourcesaved };
    }
    return out;
  });
  /** Field values with bound fields showing the source's current value (else the layer's own). */
  const fieldValues = $derived.by(() => {
    if (!tl) return {};
    if (!tl.bindings) return tl.fields;
    const out = { ...tl.fields };
    for (const [key, b] of Object.entries(tl.bindings)) {
      if (b.column === "") continue; // a list shows its binding, not a value
      if (b.row && b.row !== "current") continue; // a schedule's NOW / NEXT: the server resolves it
      const v = sourceById.get(b.sourceId)?.current[b.column];
      if (v !== undefined) out[key] = v;
    }
    return out;
  });

  const uid = $props.id();
  const size = $derived({ w: screen.width, h: screen.height });

  const templateOptions = $derived([
    ...(missing && tl ? [{ value: tl.templateId, label: "Template missing", disabled: true }] : []),
    ...templates.map((t) => ({ value: t.id, label: t.name })),
  ]);

  /** The variant shown: the layer's, else the template's default. */
  const variant = $derived(tl?.variant || template?.defaultVariant || "");
  const variantOpts = $derived.by(() => {
    if (!template) return [];
    const opts = variantOptions(template);
    return opts.some((o) => o.value === variant) ? opts : [...opts, { value: variant, label: `Missing (${variant})`, disabled: true }];
  });

  const values = $derived(template && tl ? effectiveVars(template.variables, tl.vars) : {});
  const errors = $derived(template && tl ? overrideErrors(template.variables, tl.vars) : []);
  const full = $derived(sameRect(layer.rect, fullRect(size)));

  /**
   * With the aspect lock, a new W sets H (and back): at the graphic's ratio for a layer that scales
   * (the design box's), else at the box's. W / H are a resize (the editor may switch to fit).
   */
  function setRect(patch: Partial<Rect>) {
    const ratio = tl && scalesOnResize(tl.sizing) ? graphicRatio(layer.rect, template?.design) : undefined;
    const r = clampRect(sizeRect(layer.rect, patch, aspectLocked, LAYER_SIZE_MIN, ratio), size);
    if (!sameRect(r, layer.rect)) onrect(r, patch.w !== undefined || patch.h !== undefined);
  }

  const SIZING_OPTIONS: { value: ContentSizing; label: string }[] = [
    { value: "fit", label: "Scale to fit" },
    { value: "reflow", label: "Reflow" },
  ];
  const sizing = $derived<ContentSizing>(tl?.sizing ?? "reflow");

  /** "Fit box to graphic": the design box at the layer's position (a template with a design). */
  const fit = $derived(tl && template?.design ? fitBox(layer.rect, template.design, screen) : null);
  const fitted = $derived(fit !== null && sizing === "fit" && sameRect(fit, layer.rect));
  /** Scale to fit: the graphic's scale and the box at 100 %. */
  const scale = $derived(tl && sizing === "fit" ? fitScale(layer.rect, template?.design, screen) : null);
  const actual = $derived(scale === null ? null : actualSize(layer.rect, template?.design, screen));
  /** The preset it was made from ships a newer look (keyed on that, not on a missing design). */
  const summary = $derived(tl ? templates.find((t) => t.id === tl.templateId) : undefined);

</script>

<div class="inspector" bind:this={root}>
  <section class="block" aria-labelledby="{uid}-layer">
    <h2 class="section-label" id="{uid}-layer">Layer</h2>
    <div class="grid">
      <label for="{uid}-name">Name</label>
      <span class="input vf vf-within">
        <input
          id="{uid}-name"
          type="text"
          autocomplete="off"
          maxlength={LAYER_NAME_MAX}
          value={layer.name}
          aria-invalid={layer.name.trim() === "" ? "true" : undefined}
          oninput={(e) => onname(e.currentTarget.value)}
        />
      </span>

      {#if tl}
      <label for="{uid}-tpl">Template</label>
      <Select
        id="{uid}-tpl"
        class="grow"
        value={tl.templateId}
        options={templateOptions}
        onchange={(v: string) => {
          if (v !== tl.templateId) ontemplate(v);
        }}
      />
      {/if}

      {#if variantOpts.length > 1}
        <label for="{uid}-variant">Variant</label>
        <Select
          id="{uid}-variant"
          class="grow"
          value={variant}
          options={variantOpts}
          onchange={(v: string) => {
            if (v !== variant) onvariant(v);
          }}
        />
        {#if onstyles}
          <button type="button" class="styles-link vf" onclick={onstyles}>Browse styles…</button>
        {/if}
      {/if}

      <label for="{uid}-opacity">Opacity</label>
      <OpacityInput id="{uid}-opacity" value={layer.opacity ?? 100} onvalue={onopacity} />
    </div>
    {#if missing && tl}
      <p class="warn">This layer's template no longer exists. Choose another template to show it.</p>
    {:else if variantMissing && tl?.variant}
      <p class="warn">Variant “{tl.variant}” no longer exists, so the default shows. Choose a variant.</p>
    {/if}
  </section>

  <section class="block" aria-labelledby="{uid}-box">
    <div class="block-head">
      <h2 class="section-label" id="{uid}-box">Position and size</h2>
      <span class="spacer"></span>
      <IconButton
        size="sm"
        label={aspectLocked ? "Unlock aspect ratio" : "Lock aspect ratio"}
        pressed={aspectLocked}
        pressedFill={false}
        tooltip="left"
        onclick={onlockaspect}
      >
        {#if aspectLocked}<Lock size={14} />{:else}<LockOpen size={14} />{/if}
      </IconButton>
      <Button size="sm" variant="ghost" disabled={full} onclick={() => onrect(fullRect(size))}>
        {#snippet icon()}<Maximize size={12} />{/snippet}
        Fill screen
      </Button>
    </div>
    <div class="rect">
      <label class="coord">
        <span>X</span>
        <NumberInput value={layer.rect.x} digits={5} suffix="px" aria-label="X" onchange={(n) => setRect({ x: n })} />
      </label>
      <label class="coord">
        <span>Y</span>
        <NumberInput value={layer.rect.y} digits={5} suffix="px" aria-label="Y" onchange={(n) => setRect({ y: n })} />
      </label>
      <label class="coord">
        <span>W</span>
        <NumberInput
          value={layer.rect.w}
          min={Math.min(LAYER_SIZE_MIN, layer.rect.w)}
          digits={5}
          suffix="px"
          aria-label="Width"
          onchange={(n) => setRect({ w: n })}
        />
      </label>
      <label class="coord">
        <span>H</span>
        <NumberInput
          value={layer.rect.h}
          min={Math.min(LAYER_SIZE_MIN, layer.rect.h)}
          digits={5}
          suffix="px"
          aria-label="Height"
          onchange={(n) => setRect({ h: n })}
        />
      </label>
    </div>
    {#if tl}
    <div class="grid">
      <label for="{uid}-sizing">Content sizing</label>
      <Select
        id="{uid}-sizing"
        class="grow"
        value={sizing}
        options={SIZING_OPTIONS}
        aria-describedby="{uid}-sizing-hint"
        onchange={(v: ContentSizing) => {
          if (v !== sizing) onsizing(v);
        }}
      />
      <p class="muted hint" id="{uid}-sizing-hint">
        {sizing === "fit"
          ? "Scales the design to the box."
          : tl?.sizing
            ? "The template lays out in the box."
            : "The template lays out in the box. Resizing the layer scales it."}
      </p>
    </div>
    {/if}
    {#if scale !== null && actual}
      <div class="scale">
        <span class="muted">Scale <span class="mono" data-scale>{scaleText(scale)}</span></span>
        <Button
          size="sm"
          variant="ghost"
          aria-label="Scale to 100 %"
          disabled={sameRect(actual, layer.rect)}
          onclick={() => actual && onrect(actual, true)}
        >
          100 %
        </Button>
      </div>
    {:else if fit}
      <Button size="sm" variant="ghost" class="fit-box" disabled={fitted} onclick={() => fit && onfit(fit)}>
        {#snippet icon()}<Scan size={12} />{/snippet}
        Fit box to graphic
      </Button>
    {/if}
    {#if summary?.presetUpdate}
      <!-- An outdated preset template: the notice is its own small chunk. -->
      {#await import("../../templates/UpdateNotice.svelte") then { default: UpdateNotice }}
        <UpdateNotice
          target={summary}
          message={template?.design ? undefined : "This graphic doesn't follow its box."}
          label={template?.design ? undefined : "Update to the latest look"}
        />
      {/await}
    {/if}
    <AlignBar count={1} {onalign} ondistribute={() => {}} />
    <AnchorPad anchor={layer.anchor ?? null} {onanchor} />
  </section>

  {#if layer.kind === "media"}
    {#await loadMedia() then { default: MediaInspector }}
      <MediaInspector {layer} {screenId} {live} {missing} {onpatch} />
    {/await}
  {:else if template && tl}
    <section class="block" aria-labelledby="{uid}-fields">
      <h2 class="section-label" id="{uid}-fields">Fields</h2>
      {#if template.fields.length === 0}
        <p class="muted">“{template.name}” has no fields.</p>
      {:else}
        <FieldInputs defs={template.fields} values={fieldValues} bound={boundLabels} {linked} {onfield} {ontimer}>
          {#snippet extra(f)}
            <!-- Clocks are set in the template: no binding. Timers bind a schedule's countdown only. -->
            {#if f.type !== "clock"}
              {@const pick = (b: SourceBinding) => {
                onbind(f.key, b);
                void refocus(f.key);
              }}
              <span class="bindctl" data-bindctl={f.key}>
                {#if f.type === "list"}
                  {#await loadListBind() then { default: ListBind }}
                    <ListBind
                      field={f.label || f.key}
                      columns={f.columns ?? []}
                      {sources}
                      current={tl.bindings?.[f.key] ?? null}
                      onopen={onloadsources}
                      onpick={pick}
                    />
                  {/await}
                {/if}
                {#if boundLabels[f.key] !== undefined}
                  <IconButton
                    size="sm"
                    label="{f.type === 'list' ? 'Unlink' : 'Unbind'} {f.label || f.key}"
                    tooltip="left"
                    onclick={() => {
                      onunbind(f.key);
                      void refocus(f.key);
                    }}
                  >
                    <Unlink size={14} />
                  </IconButton>
                {:else if f.type !== "list"}
                  {#await bindMenu then { default: BindMenu }}
                    <BindMenu
                      field={f.label || f.key}
                      {sources}
                      countdown={f.type === "timer"}
                      onopen={onloadsources}
                      onpick={pick}
                    />
                  {/await}
                {/if}
              </span>
            {/if}
          {/snippet}
        </FieldInputs>
      {/if}
      {#each sourceWarnings as w, i (i)}
        <p class="warn">Source missing: {w}</p>
      {/each}
    </section>

    <section class="block" aria-labelledby="{uid}-vars">
      <h2 class="section-label" id="{uid}-vars">Variables</h2>
      {#if template.variables.length === 0}
        <p class="muted">“{template.name}” has no variables.</p>
      {:else}
        <VariableValuesForm
          variables={template.variables}
          {values}
          overrides={tl.vars}
          {errors}
          onvalue={(key, value) => onvar(key, value)}
          onreset={onresetvar}
        />
      {/if}
    </section>
  {/if}
</div>

<style>
  .inspector {
    display: flex;
    flex-direction: column;
  }
  .block {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding: var(--s-4);
    border-bottom: 1px solid var(--line);
  }
  .block-head {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .spacer {
    flex: 1;
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(5rem, auto) minmax(0, 1fr);
    gap: var(--s-2) var(--s-3);
    align-items: center;
  }
  .grid > label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .styles-link {
    grid-column: 2;
    justify-self: start;
    height: var(--control-h-sm);
    margin-top: calc(var(--s-1) * -1);
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    color: var(--accent-text);
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .styles-link:hover {
    background: var(--surface-2);
  }
  .grid :global(.grow) {
    width: 100%;
  }
  .block > :global(.fit-box) {
    align-self: flex-start;
  }
  .input {
    display: flex;
    min-width: 0;
    border-radius: var(--r-control);
  }
  .input input {
    width: 100%;
    min-width: 0;
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .input input {
    height: var(--control-h);
  }
  input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }

  .rect {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--s-2) var(--s-3);
  }
  .coord {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .coord > span {
    width: 1ch;
  }

  .scale {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-2);
  }
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .hint {
    grid-column: 2;
    margin-top: calc(var(--s-1) * -1);
  }
  .bindctl {
    display: inline-flex;
    flex: none;
  }
  .warn {
    color: var(--warning);
    font-size: var(--fs-sm);
  }
</style>

<!--
  The rows of Preferences > Style & brand (lazy chunk, loaded by ./StyleSection.svelte), with a
  small live preview of the built-in lower third beside them (above them on narrow layouts):

    Style        the project's style (`list_styles` that some preset ships, Classic too). Choosing one is "Apply to
                 project" ($lib/components/styles/apply: asks first when layers on air would
                 switch; a result toast with Undo). Canceled or failed: the select snaps back.
    Brand kit    accent, secondary, text and plate colors (swatch + hex + "Transparent"; transparent
                 shows the checkerboard, own or inherited; turning it off restores the last own
                 color, else the style's), heading and body fonts,
                 the logo (the asset picker). An unset value shows what the style gives it, dimmed
                 with the style's name; picking or typing a value overrides it, "Reset" unsets.
                 Saved on change (`save_brand_kit`), optimistically: a rejected save rolls the
                 form back and shows the reason under the row. Typed hex is checked first and
                 saved after a short pause; the swatch saves when its picker closes.
    Preview      ./brandPreview.ts (its own lazy chunk, imported on mount): the lower third in the
                 style with the values shown in the form (typed and dragged ones too), redrawn
                 150 ms after the last change. The inherited values come from the same look.
  No props.
-->
<script lang="ts">
  import RotateCcw from "lucide-svelte/icons/rotate-ccw";
  import { toAppError } from "$lib/api";
  import { listPresets, listStyles, saveBrandKit } from "$lib/api/templates";
  import Button from "$lib/components/Button.svelte";
  import Select from "$lib/components/Select.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import { thumbUrl } from "$lib/components/assets/assets";
  import ImageChoose from "$lib/components/fields/ImageChoose.svelte";
  import { offered } from "$lib/components/styles/styles";
  import { project } from "$lib/stores/project.svelte";
  import { server } from "$lib/stores/server.svelte";
  import type { BrandKit, StyleInfo } from "$lib/types";
  import TransparentToggle from "$lib/components/variables/TransparentToggle.svelte";
  import { alphaOf as alphaSuffix } from "$lib/components/variables/color";
  import {
    BRAND_COLORS,
    BRAND_FONTS,
    clean,
    colorIssue,
    fontOptions,
    isHex,
    isTransparent,
    normalizeColor,
    pickerHex,
    withValue,
    type BrandColor,
    type BrandKey,
  } from "./brand";
  import type { BrandPreview, StyleLook } from "./brandPreview";

  const uid = $props.id();
  /** Typed hex waits this long before it is saved. */
  const TYPE_MS = 400;

  let styles = $state.raw<StyleInfo[]>([]);
  const saved = $derived(clean(project.current?.settings.brand));
  const projectStyle = $derived(project.current?.settings.style ?? "");
  const styleName = $derived(styles.find((s) => s.id === projectStyle)?.name ?? "");

  /** The form's values: the saved kit, then the user's edits until a save answers. */
  let brand = $derived<BrandKit>(saved);
  /** Text typed into a hex field (it may be invalid or half done). */
  let typed = $state<Partial<Record<BrandKey, string>>>({});
  let errors = $state<Partial<Record<BrandKey, string>>>({});
  // Writable derived: follows the project, snaps back when an apply is canceled or fails.
  let style = $derived(projectStyle);
  let applying = $state(false);

  const styleOptions = $derived([
    ...(projectStyle === "" ? [{ value: "", label: "None", disabled: true }] : []),
    ...styles.map((s) => ({ value: s.id, label: s.name })),
  ]);

  $effect(() => server.watch());

  $effect(() => {
    let alive = true;
    Promise.all([listStyles(), listPresets().catch(() => [])]).then(
      ([list, presets]) => {
        if (alive) styles = offered(list, presets);
      },
      () => {},
    );
    return () => {
      alive = false;
    };
  });

  // ---- The style's look: inherited values and the preview ----

  type PreviewModule = typeof import("./brandPreview");
  let previewModule = $state.raw<PreviewModule | null>(null);
  let look = $state.raw<StyleLook | null>(null);
  let previewEl = $state<HTMLElement>();
  let preview = $state.raw<BrandPreview | null>(null);
  // The previous style's values stay until the new style's look has loaded (no flash of empty).
  const inherited = $derived(look?.inherited ?? {});

  $effect(() => {
    let alive = true;
    import("./brandPreview").then(
      (m) => {
        if (alive) previewModule = m;
      },
      () => {},
    );
    return () => {
      alive = false;
    };
  });

  $effect(() => {
    const m = previewModule;
    const s = projectStyle;
    if (!m) return;
    let alive = true;
    m.loadStyleLook(s).then(
      (l) => {
        if (alive) look = l;
      },
      () => {},
    );
    return () => {
      alive = false;
    };
  });

  $effect(() => {
    const m = previewModule;
    const el = previewEl;
    if (!m || !el) return;
    const p = new m.BrandPreview(el);
    preview = p;
    return () => {
      p.destroy();
      if (preview === p) preview = null;
    };
  });

  /** What the preview shows: the form's values, a complete typed or dragged hex included. */
  const shown = $derived.by((): BrandKit => {
    const out: BrandKit = { ...brand };
    for (const c of BRAND_COLORS) {
      const t = typed[c.key]?.trim();
      if (t && colorIssue(t) === null) out[c.key] = normalizeColor(t);
    }
    return out;
  });
  const running = $derived(server.status?.state === "running" ? server.status.url : null);

  $effect(() => {
    preview?.set(look, {
      brand: shown,
      logo: brand.logo ? thumbUrl(server.status, brand.logo) : null,
      fontBase: running,
    });
  });

  async function applyStyle(id: string) {
    const s = styles.find((x) => x.id === id);
    if (!s || applying) return;
    applying = true;
    try {
      const { applyToProject } = await import("$lib/components/styles/apply");
      if (!(await applyToProject(s))) style = projectStyle;
    } catch {
      style = projectStyle;
    } finally {
      applying = false;
    }
  }

  // ---- Brand kit: serialized, optimistic saves ----

  let chain: Promise<unknown> = Promise.resolve();
  const timers = new Map<BrandKey, ReturnType<typeof setTimeout>>();

  $effect(() => () => {
    for (const t of timers.values()) clearTimeout(t);
    timers.clear();
  });

  function save(key: BrandKey, value: string | null) {
    const t = timers.get(key);
    if (t !== undefined) clearTimeout(t);
    timers.delete(key);
    brand = withValue(brand, key, value);
    const next = brand;
    errors = { ...errors, [key]: undefined };
    chain = chain.then(async () => {
      try {
        const result = clean(await saveBrandKit(next));
        const p = project.current;
        if (p) project.replace({ ...p, settings: { ...p.settings, brand: result } });
        brand = result;
      } catch (e) {
        brand = saved;
        typed = { ...typed, [key]: undefined };
        errors = { ...errors, [key]: toAppError(e).message };
      }
    });
    return chain;
  }

  function onHexInput(key: BrandKey, raw: string) {
    const v = normalizeColor(raw.trim());
    typed = { ...typed, [key]: raw };
    const issue = colorIssue(v);
    errors = { ...errors, [key]: issue ?? undefined };
    const t = timers.get(key);
    if (t !== undefined) clearTimeout(t);
    timers.delete(key);
    if (issue || v === (brand[key] ?? "")) return;
    timers.set(
      key,
      setTimeout(() => {
        timers.delete(key);
        void save(key, v || null);
      }, TYPE_MS),
    );
  }

  function onHexBlur(key: BrandKey) {
    // Saved already (or pending): show the canonical value again unless it's still being fixed.
    if (!errors[key]) typed = { ...typed, [key]: undefined };
  }

  function reset(key: BrandKey) {
    typed = { ...typed, [key]: undefined };
    void save(key, null);
  }

  /** The alpha part (`aa` of #rrggbbaa) of the value shown, kept when the swatch picks a color. */
  function alphaOf(key: BrandKey): string {
    return alphaSuffix(brand[key] ?? (key === "logo" ? "" : inherited[key]) ?? "");
  }

  /** The last own hex color per key before it was made transparent (what turning it off restores). */
  const lastHex = new Map<BrandKey, string>();

  function toggleTransparent(key: BrandColor, own: string | null | undefined) {
    typed = { ...typed, [key]: undefined };
    if (isTransparent(own)) {
      void save(key, lastHex.get(key) ?? null);
    } else {
      if (own && isHex(own)) lastHex.set(key, own);
      void save(key, "transparent");
    }
  }

  /** Swatch dragging: shown at once (as typed text), saved when the picker closes. */
  function onSwatchInput(key: BrandKey, hex: string) {
    typed = { ...typed, [key]: hex + alphaOf(key) };
  }

  function onSwatchChange(key: BrandKey, hex: string) {
    const v = hex + alphaOf(key);
    typed = { ...typed, [key]: undefined };
    void save(key, v);
  }
</script>

{#snippet resetButton(key: BrandKey, label: string)}
  <Button size="sm" variant="ghost" aria-label="Reset {label} to the style" onclick={() => reset(key)}>
    {#snippet icon()}<RotateCcw size={12} />{/snippet}
    Reset
  </Button>
{/snippet}

{#snippet styleTag()}
  {#if styleName}<span class="tag"><span class="visually-hidden">From the style </span>{styleName}</span>{/if}
{/snippet}

{#snippet error(key: BrandKey)}
  {#if errors[key]}<p class="err" id="{uid}-{key}-err" role="alert">{errors[key]}</p>{/if}
{/snippet}

<div class="style-brand">
  <div class="pv">
    <div class="frame checker-dark" role="img" aria-label="Lower third preview">
      <span class="thumb" bind:this={previewEl}></span>
    </div>
  </div>

  <div class="rows">
    <SettingsRow label="Style" description="Applies to all built-in graphics." for="{uid}-style">
      <Select
        id="{uid}-style"
        class="grow"
        value={style}
        options={styleOptions}
        disabled={applying || styles.length === 0}
        onchange={(v: string) => {
          style = v;
          void applyStyle(v);
        }}
      />
    </SettingsRow>

    {#each BRAND_COLORS as c (c.key)}
      {@const own = brand[c.key] !== undefined || typed[c.key] !== undefined}
      {@const from = inherited[c.key] ?? ""}
      {@const value = typed[c.key] ?? brand[c.key] ?? ""}
      {@const clear = isTransparent((own ? value : from).trim())}
      <SettingsRow label="{c.label} color" for="{uid}-{c.key}">
        <div class="cell">
          <div class="line">
            <span class="swatch vf vf-within" class:inherited={!own} class:unknown={!own && !from} class:checker={clear} class:clear>
              <input
                type="color"
                aria-label="{c.label} color picker"
                value={pickerHex(own ? value : from)}
                oninput={(e) => onSwatchInput(c.key, e.currentTarget.value)}
                onchange={(e) => onSwatchChange(c.key, e.currentTarget.value)}
              />
            </span>
            <span class="text vf vf-within">
              <input
                id="{uid}-{c.key}"
                class="mono"
                type="text"
                spellcheck="false"
                autocomplete="off"
                maxlength="11"
                {value}
                placeholder={from}
                aria-invalid={errors[c.key] ? "true" : undefined}
                aria-describedby={errors[c.key] ? `${uid}-${c.key}-err` : undefined}
                oninput={(e) => onHexInput(c.key, e.currentTarget.value)}
                onblur={() => onHexBlur(c.key)}
              />
            </span>
            <TransparentToggle
              pressed={own && isTransparent(value.trim())}
              label="{c.label} color"
              ontoggle={() => toggleTransparent(c.key, brand[c.key])}
            />
            {#if brand[c.key]}{@render resetButton(c.key, c.label)}{:else if !own}{@render styleTag()}{/if}
          </div>
          {@render error(c.key)}
        </div>
      </SettingsRow>
    {/each}

    {#each BRAND_FONTS as f (f.key)}
      <SettingsRow label={f.label} for="{uid}-{f.key}">
        <div class="cell">
          <div class="line">
            <Select
              id="{uid}-{f.key}"
              class="grow"
              value={brand[f.key] ?? ""}
              options={fontOptions(inherited[f.key], styleName)}
              onchange={(v: string) => void save(f.key, v || null)}
            />
            {#if brand[f.key]}{@render resetButton(f.key, f.label)}{/if}
          </div>
          {@render error(f.key)}
        </div>
      </SettingsRow>
    {/each}

    <SettingsRow label="Logo" description="Used when a graphic has no logo of its own.">
      <div class="cell">
        <div class="line">
          <ImageChoose label="Logo" value={brand.logo ?? ""} onchange={(v) => void save("logo", v || null)} />
          {#if brand.logo}
            <span class="mono path">{brand.logo}</span>
          {:else}
            <span class="muted">None</span>
          {/if}
        </div>
        {@render error("logo")}
      </div>
    </SettingsRow>
  </div>
</div>

<style>
  /* Wide: rows left, the preview right of them. Narrow (the section is the size container):
     the preview above the rows. */
  .style-brand {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 320px;
    column-gap: var(--s-6);
  }
  .rows {
    grid-row: 1;
    grid-column: 1;
    min-width: 0;
  }
  .pv {
    grid-row: 1;
    grid-column: 2;
    padding-top: var(--s-2);
    border-top: 1px solid var(--line);
  }
  .frame {
    position: sticky;
    top: var(--s-4);
    overflow: hidden;
    border-radius: var(--r-control);
  }
  .thumb {
    display: block;
    width: 100%;
    aspect-ratio: 16 / 9;
  }
  @container (max-width: 880px) {
    .style-brand {
      grid-template-columns: minmax(0, 1fr);
    }
    .pv {
      grid-column: 1;
      padding-bottom: var(--s-2);
    }
    .frame {
      position: static;
      width: min(320px, 100%);
    }
    .rows {
      grid-row: 2;
    }
  }

  .cell {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    min-width: 0;
    width: 100%;
  }
  .line {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .swatch {
    --vf-radius: var(--r-control);
    position: relative;
    display: inline-flex;
    flex: none;
    width: var(--control-h);
    height: var(--control-h);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .swatch input {
    width: 100%;
    height: 100%;
    padding: 3px;
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    cursor: pointer;
  }
  /* Inherited from the style: the style's color, dimmed. Still unknown (loading): empty. */
  .swatch.inherited input {
    opacity: 0.45;
  }
  .swatch.unknown input {
    opacity: 0;
  }
  /* Transparent (own or inherited): the checkerboard shows through the (still clickable) picker. */
  .swatch.clear input {
    opacity: 0;
  }
  .text {
    --vf-radius: var(--r-control);
    display: flex;
    width: 120px;
    height: var(--control-h);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .text input {
    flex: 1;
    min-width: 0;
    padding: 0 var(--s-2);
    border: 0;
    background: transparent;
  }
  .text input::placeholder {
    color: var(--text-muted);
    opacity: 1;
  }
  .text:has(input[aria-invalid="true"]) {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .tag {
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text-muted);
    font-size: var(--fs-sm);
    line-height: 20px;
    white-space: nowrap;
  }
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .path {
    min-width: 0;
    overflow: hidden;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .err {
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  .line > :global(.grow) {
    flex: 1;
    min-width: 0;
  }
</style>

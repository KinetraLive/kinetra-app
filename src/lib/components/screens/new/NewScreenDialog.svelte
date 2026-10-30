<!--
  "New screen" dialog: a native modal <dialog> (focus trap, Esc, inert background). Name,
  resolution (presets or Custom) and background (Transparent / Colour). Create: the screen is
  created, the dialog closes and the layout editor opens (`/screens/<id>`). A failed create shows
  the error inline and keeps the dialog open.

  Lazy chunk: open it with `NewScreenLauncher` / `loadNewScreenDialog()` from ./launcher.svelte.ts.

  Props:
    open: boolean          shows the dialog (showModal); false closes it
    onclose: () => void    Esc, Cancel, a backdrop click, or after Create. The parent sets open=false.
    names?: string[]       existing screen names, for the default "Screen N" (loaded when omitted)

  Defaults: the project's default resolution (a preset when it matches one, else Custom), a
  transparent background. Focus goes to the name on open and back to the opener on close.
-->
<script lang="ts">
  import { backdropClose } from "$lib/components/backdrop";
  import { tick } from "svelte";
  import { goto } from "$app/navigation";
  import { toAppError } from "$lib/api";
  import { createScreen, listScreenDocs } from "$lib/api/screens";
  import Button from "$lib/components/Button.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import { project } from "$lib/stores/project.svelte";
  import { SCREEN_NAME_MAX, SCREEN_SIZE_RANGE } from "$lib/types";
  import {
    RESOLUTION_PRESETS,
    defaultScreenName,
    presetFor,
    validHex,
    validScreenName,
    validSide,
    type PresetKey,
  } from "./format";
  import type { NewScreenProps } from "./launcher.svelte";

  let { open, onclose, names }: NewScreenProps = $props();

  const uid = $props.id();
  const FALLBACK = { width: 1920, height: 1080 };

  let dialog: HTMLDialogElement | undefined = $state();
  let nameInput: HTMLInputElement | undefined = $state();
  let name = $state("");
  /** The user typed a name: a late-loading screen list no longer replaces it. */
  let nameTouched = false;
  let preset = $state<PresetKey>("1080p");
  let customW = $state(FALLBACK.width);
  let customH = $state(FALLBACK.height);
  let bg = $state<"transparent" | "color">("transparent");
  let hex = $state("#000000");
  let error = $state<string | null>(null);
  let creating = $state(false);
  let returnFocus: HTMLElement | null = null;
  let gen = 0;

  const size = $derived.by(() => {
    const p = RESOLUTION_PRESETS.find((x) => x.key === preset);
    return p ? { width: p.width, height: p.height } : { width: customW, height: customH };
  });
  const sizeOk = $derived(validSide(size.width) && validSide(size.height));
  const hexOk = $derived(bg === "transparent" || validHex(hex));
  const canCreate = $derived(validScreenName(name) && sizeOk && hexOk && !creating);
  /** What the native colour input shows: the hex's RGB part (it has no alpha). */
  const swatch = $derived(validHex(hex) ? hex.trim().slice(0, 7).toLowerCase() : "#000000");

  function reset() {
    const def = project.current?.settings.defaultResolution ?? FALLBACK;
    preset = presetFor(def);
    customW = def.width;
    customH = def.height;
    bg = "transparent";
    hex = "#000000";
    error = null;
    nameTouched = false;
    name = defaultScreenName(names ?? []);
    if (!names) {
      const g = ++gen;
      listScreenDocs().then(
        (list) => {
          if (g === gen && !nameTouched) name = defaultScreenName(list.map((s) => s.name));
        },
        () => {
          // Keep "Screen 1"; the name is editable anyway.
        },
      );
    }
  }

  // Open / close the native dialog; focus the name on open, give focus back on close.
  $effect(() => {
    const d = dialog;
    if (!d) return;
    if (open && !d.open) {
      returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      reset();
      if (typeof d.showModal === "function") d.showModal();
      else d.setAttribute("open", "");
      void tick().then(() => {
        nameInput?.focus();
        nameInput?.select();
      });
    } else if (!open && d.open) {
      shut(d);
    }
  });

  // Unmounted while open: still hand focus back.
  $effect(() => () => {
    gen++;
    if (dialog?.open) shut(dialog);
  });

  function shut(d: HTMLDialogElement) {
    gen++;
    if (typeof d.close === "function") d.close();
    else d.removeAttribute("open");
    const el = returnFocus;
    returnFocus = null;
    if (el?.isConnected) el.focus();
  }

  function cancel() {
    if (!creating) onclose();
  }

  function onCancelEvent(e: Event) {
    // The parent owns visibility; it closes us from onclose.
    e.preventDefault();
    cancel();
  }


  async function create(e?: SubmitEvent) {
    e?.preventDefault();
    if (!canCreate) return;
    creating = true;
    error = null;
    let id: string;
    try {
      const state = await createScreen({
        name: name.trim(),
        width: size.width,
        height: size.height,
        background: bg === "transparent" ? "transparent" : hex.trim().toLowerCase(),
      });
      id = state.doc.id;
    } catch (err) {
      error = toAppError(err).message;
      return;
    } finally {
      creating = false;
    }
    onclose();
    await goto(`/screens/${id}`);
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialog}
  class="dialog"
  aria-labelledby="{uid}-title"
  oncancel={onCancelEvent}
  {@attach backdropClose(cancel)}
>
  <form class="inner" onsubmit={create} novalidate>
    <h2 id="{uid}-title" class="title">New screen</h2>

    <div class="field">
      <label for="{uid}-name">Name</label>
      <div class="input vf vf-within">
        <input
          id="{uid}-name"
          type="text"
          bind:this={nameInput}
          bind:value={name}
          oninput={() => (nameTouched = true)}
          maxlength={SCREEN_NAME_MAX}
          required
          autocomplete="off"
          spellcheck="false"
          disabled={creating}
        />
      </div>
    </div>

    <fieldset class="group">
      <legend>Resolution</legend>
      <div class="segs">
        {#each RESOLUTION_PRESETS as p (p.key)}
          <label class="seg vf vf-within">
            <input
              class="visually-hidden"
              type="radio"
              name="{uid}-res"
              value={p.key}
              bind:group={preset}
              disabled={creating}
            />
            <span class="seg-value mono">{p.width}×{p.height}</span>
            <span class="seg-caption">{p.label}</span>
          </label>
        {/each}
        <label class="seg vf vf-within">
          <input
            class="visually-hidden"
            type="radio"
            name="{uid}-res"
            value="custom"
            bind:group={preset}
            disabled={creating}
          />
          <span class="seg-value">Custom</span>
          <span class="seg-caption">Any size</span>
        </label>
      </div>

      {#if preset === "custom"}
        <div class="custom">
          <NumberInput
            bind:value={customW}
            digits={4}
            suffix="px"
            aria-label="Width"
            aria-invalid={!validSide(customW)}
            aria-describedby={sizeOk ? undefined : `${uid}-size-err`}
            disabled={creating}
          />
          <span class="times mono" aria-hidden="true">×</span>
          <NumberInput
            bind:value={customH}
            digits={4}
            suffix="px"
            aria-label="Height"
            aria-invalid={!validSide(customH)}
            aria-describedby={sizeOk ? undefined : `${uid}-size-err`}
            disabled={creating}
          />
        </div>
        {#if !sizeOk}
          <p id="{uid}-size-err" class="error">
            Width and height must be whole numbers from <span class="mono"
              >{SCREEN_SIZE_RANGE.min}</span
            >
            to <span class="mono">{SCREEN_SIZE_RANGE.max}</span> px.
          </p>
        {/if}
      {/if}
    </fieldset>

    <fieldset class="group">
      <legend>Background</legend>
      <div class="segs bg">
        <label class="seg vf vf-within">
          <input
            class="visually-hidden"
            type="radio"
            name="{uid}-bg"
            value="transparent"
            bind:group={bg}
            disabled={creating}
          />
          <span class="seg-value">Transparent</span>
        </label>
        <label class="seg vf vf-within">
          <input
            class="visually-hidden"
            type="radio"
            name="{uid}-bg"
            value="color"
            bind:group={bg}
            disabled={creating}
          />
          <span class="seg-value">Color</span>
        </label>
      </div>

      {#if bg === "color"}
        <div class="colour">
          <span class="swatch vf vf-within">
            <input
              type="color"
              aria-label="Background color"
              value={swatch}
              oninput={(e) => (hex = e.currentTarget.value)}
              disabled={creating}
            />
          </span>
          <div class="input hex vf vf-within">
            <input
              class="mono"
              type="text"
              aria-label="Background color code"
              bind:value={hex}
              maxlength={9}
              autocomplete="off"
              spellcheck="false"
              aria-invalid={!hexOk}
              aria-describedby={hexOk ? undefined : `${uid}-hex-err`}
              disabled={creating}
            />
          </div>
        </div>
        {#if !hexOk}
          <p id="{uid}-hex-err" class="error">
            Enter a color like <span class="mono">#1a1a1a</span>.
          </p>
        {/if}
      {/if}
    </fieldset>

    {#if error}<p class="error" role="alert">{error}</p>{/if}

    <div class="actions">
      <Button variant="ghost" onclick={cancel} disabled={creating}>Cancel</Button>
      <Button variant="primary" type="submit" disabled={!canCreate && !creating} loading={creating}>
        Create
      </Button>
    </div>
  </form>
</dialog>

<style>
  .dialog {
    width: min(600px, calc(100vw - 2 * var(--s-6)));
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
    gap: var(--s-5);
    padding: var(--s-5);
  }
  .title {
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }

  .field {
    display: grid;
    gap: var(--s-1);
  }
  label,
  legend {
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
  .input input {
    flex: 1;
    min-width: 0;
    padding: 0 var(--s-3);
    border: 0;
    background: transparent;
  }
  input:disabled {
    opacity: 0.5;
  }

  .group {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }
  legend {
    margin-bottom: var(--s-1);
    padding: 0;
  }
  .segs {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(96px, 1fr));
    gap: var(--s-2);
  }
  .segs.bg {
    grid-template-columns: repeat(2, minmax(0, 140px));
  }
  .seg {
    --vf-radius: var(--r-control);
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    min-height: var(--control-h);
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text);
    cursor: pointer;
    user-select: none;
  }
  .seg:hover {
    background: var(--surface-3);
  }
  /* Checked: the same look as a selected .vf card (subtle surface + 1px accent border). */
  .seg:has(input:checked) {
    background: var(--selected-bg);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .seg:has(input:disabled) {
    cursor: default;
    opacity: 0.5;
  }
  .seg-value {
    font-size: var(--fs-md);
    font-weight: 500;
  }
  .seg-caption {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }

  .custom,
  .colour {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .times {
    color: var(--text-muted);
  }
  .swatch {
    --vf-radius: var(--r-control);
    display: inline-flex;
    width: var(--control-h);
    height: var(--control-h);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .swatch input {
    width: 100%;
    height: 100%;
    padding: 2px;
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    cursor: pointer;
  }
  .hex {
    width: calc(9ch + 2 * var(--s-3));
  }
  .error {
    color: var(--danger);
    font-size: var(--fs-sm);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--s-2);
    padding-top: var(--s-4);
    border-top: 1px solid var(--line);
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
</style>

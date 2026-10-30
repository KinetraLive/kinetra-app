<!--
  "New flow" dialog (lazy chunk): a native modal <dialog> that starts Blank or From a template.
  Blank (the default) asks for the name ("Flow N" by default); From a template closes it and opens
  the flow template gallery (`ontemplate`).
  Create makes an armed flow with one Cue trigger (deployed at once, so its cue button works), closes
  and opens it in the flow editor. A failed create shows the error inline and keeps the dialog open.
  Focus goes to the name on open and back to the opener on close.

  Props:
    names: string[]          existing flow names (for the default name)
    oncreate: (name) => Promise<string>   creates; resolves to the new flow's id (throws to show)
    ontemplate: () => void   "From a template": the parent closes this and opens the gallery
    onclose: () => void      Esc, Cancel, a backdrop click, or after Create
-->
<script lang="ts">
  import { backdropClose } from "$lib/components/backdrop";
  import { onMount, tick } from "svelte";
  import { goto } from "$app/navigation";
  import { toAppError } from "$lib/api";
  import Button from "$lib/components/Button.svelte";
  import { FLOW_NAME_MAX } from "$lib/types";
  import { defaultName, nameError } from "../names";

  let {
    names,
    oncreate,
    ontemplate,
    onclose,
  }: { names: string[]; oncreate: (name: string) => Promise<string>; ontemplate: () => void; onclose: () => void } =
    $props();

  const uid = $props.id();
  let dialog: HTMLDialogElement | undefined = $state();
  let input: HTMLInputElement | undefined = $state();
  // svelte-ignore state_referenced_locally
  let name = $state(defaultName("Flow", names));
  let error = $state<string | null>(null);
  let creating = $state(false);
  const invalid = $derived(nameError(name));

  onMount(() => {
    const d = dialog;
    if (!d) return;
    const back = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (typeof d.showModal === "function") d.showModal();
    else d.setAttribute("open", "");
    void tick().then(() => {
      input?.focus();
      input?.select();
    });
    return () => {
      if (typeof d.close === "function") d.close();
      else d.removeAttribute("open");
      if (back?.isConnected) back.focus();
    };
  });

  function cancel() {
    if (!creating) onclose();
  }

  async function create(e?: SubmitEvent) {
    e?.preventDefault();
    if (invalid || creating) return;
    creating = true;
    error = null;
    let id: string;
    try {
      id = await oncreate(name.trim());
    } catch (err) {
      error = toAppError(err).message;
      creating = false;
      return;
    }
    creating = false;
    onclose();
    await goto(`/flows/${id}`);
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
  <form class="inner" onsubmit={create} novalidate>
    <h2 id="{uid}-title" class="title">New flow</h2>
    <div class="start" role="group" aria-label="Start from">
      <button type="button" class="tile vf" aria-pressed="true" onclick={() => input?.focus()}>
        <span class="tile-name">Blank</span>
        <span class="tile-desc">Starts with a Cue trigger.</span>
      </button>
      <button type="button" class="tile vf" aria-pressed="false" disabled={creating} onclick={ontemplate}>
        <span class="tile-name">From a template</span>
        <span class="tile-desc">A ready-made flow for your layers and sources.</span>
      </button>
    </div>
    <label for="{uid}-name">Name</label>
    <div class="input vf vf-within">
      <input
        id="{uid}-name"
        type="text"
        bind:this={input}
        bind:value={name}
        maxlength={FLOW_NAME_MAX}
        autocomplete="off"
        spellcheck="false"
        aria-invalid={invalid !== null}
        disabled={creating}
      />
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="actions">
      <Button variant="ghost" onclick={cancel} disabled={creating}>Cancel</Button>
      <Button variant="primary" type="submit" disabled={invalid !== null} loading={creating}>Create</Button>
    </div>
  </form>
</dialog>

<style>
  .dialog {
    width: min(480px, calc(100vw - 2 * var(--s-6)));
    margin: auto;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    color: var(--text);
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
    gap: var(--s-2);
    padding: var(--s-5);
  }
  .title {
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .start {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--s-2);
    margin: var(--s-1) 0 var(--s-2);
  }
  .tile {
    --vf-radius: var(--r-control);
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-1);
    padding: var(--s-2) var(--s-3);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text);
    text-align: left;
  }
  .tile:hover:not(:disabled) {
    background: var(--surface-3);
  }
  /* The chosen start: the same look as a selected .vf card. */
  .tile[aria-pressed="true"] {
    background: var(--selected-bg);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .tile-name {
    font-weight: 600;
  }
  .tile-desc {
    color: var(--text-muted);
    font-size: var(--fs-sm);
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
    justify-content: flex-end;
    gap: var(--s-2);
    margin-top: var(--s-3);
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

<!--
  "New source" dialog: a native modal <dialog> (focus trap, Esc, inert background) with the kind
  (Data table / Schedule / CSV link / API / Live data, a radio group; Simple mode hides API and
  Live data) and the name. CSV link
  is an API source set to CSV. Create makes a data table (kind `text`, one
  column "Text", no rows), a schedule (no items), an API source or a live data source (no URL yet), closes the dialog and hands the new document to
  `oncreated`. A failed create shows the error inline and keeps the dialog open.

  Lazy chunk: open it with `NewSourceLauncher` / `loadNewSourceDialog()` from ./launcher.svelte.ts.

  Props:
    open: boolean                     shows the dialog (showModal); false closes it
    onclose: () => void               Esc, Cancel, a backdrop click, or after Create
    oncreated?: (doc) => void         after Create (before onclose)
    names?: string[]                  existing source names, for "Source N" (loaded when omitted)
-->
<script lang="ts">
  import { tick } from "svelte";
  import { toAppError } from "$lib/api";
  import { createApiSource } from "$lib/api/api-sources";
  import { createSchedule, createSource, listSources, saveSource } from "$lib/api/sources";
  import Button from "$lib/components/Button.svelte";
  import { isAdvanced } from "$lib/stores/mode.svelte";
  import { SOURCE_NAME_MAX, type SourceDoc, type SourceKindName } from "$lib/types";
  import { defaultSourceName, validSourceName } from "../model";
  import type { NewSourceProps } from "./launcher.svelte";

  let { open, onclose, oncreated, names }: NewSourceProps = $props();

  const uid = $props.id();

  let dialog: HTMLDialogElement | undefined = $state();
  let nameInput: HTMLInputElement | undefined = $state();
  let name = $state("");
  /** "csv": an API source fetching CSV (a CSV file or a Google Sheets CSV link). */
  type Kind = SourceKindName | "csv";
  let kind = $state<Kind>("text");
  const ALL_KINDS: ReadonlyArray<{ value: Kind; label: string; desc: string; advanced?: boolean }> = [
    {
      value: "text",
      label: "Data table",
      desc: "Rows and columns, like a guest list.",
    },
    {
      value: "schedule",
      label: "Schedule",
      desc: "A run of show that tracks Now and Up next.",
    },
    {
      value: "csv",
      label: "CSV link",
      desc: "A CSV file or Google Sheets link.",
    },
    {
      value: "api",
      label: "API",
      desc: "JSON or CSV fetched from a URL.",
      advanced: true,
    },
    {
      value: "socket",
      label: "Live data (WebSocket)",
      desc: "JSON pushed from this computer or your network.",
      advanced: true,
    },
  ];
  /** Simple mode hides JSON APIs (CSV links stay). */
  const KINDS = $derived(isAdvanced() ? ALL_KINDS : ALL_KINDS.filter((k) => !k.advanced));
  let nameTouched = false;
  let error = $state<string | null>(null);
  let creating = $state(false);
  let returnFocus: HTMLElement | null = null;
  let gen = 0;

  const canCreate = $derived(validSourceName(name) && !creating);

  function reset() {
    error = null;
    nameTouched = false;
    kind = "text";
    name = defaultSourceName(names ?? []);
    if (!names) {
      const g = ++gen;
      listSources().then(
        (list) => {
          if (g === gen && !nameTouched) name = defaultSourceName(list.map((s) => s.name));
        },
        () => {
          // Keep "Source 1"; the name is editable anyway.
        },
      );
    }
  }

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
    e.preventDefault();
    cancel();
  }

  function onBackdropClick(e: MouseEvent) {
    if (e.target === dialog) cancel();
  }

  /** An API source set to CSV (header row, delimiter detected). */
  async function createCsvLink(n: string): Promise<SourceDoc> {
    const d = await createApiSource(n);
    return d.kind === "api" ? saveSource({ ...d, format: "csv" }) : d;
  }

  async function create(e?: SubmitEvent) {
    e?.preventDefault();
    if (!canCreate) return;
    creating = true;
    error = null;
    try {
      const n = name.trim();
      const doc =
        kind === "schedule"
          ? await createSchedule(n)
          : kind === "api"
            ? await createApiSource(n)
            : kind === "socket"
              ? await (await import("$lib/api/socket-sources")).createSocketSource(n)
            : kind === "csv"
              ? await createCsvLink(n)
              : await createSource(n);
      oncreated?.(doc);
    } catch (err) {
      error = toAppError(err).message;
      return;
    } finally {
      creating = false;
    }
    onclose();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialog}
  class="dialog"
  aria-labelledby="{uid}-title"
  oncancel={onCancelEvent}
  onclick={onBackdropClick}
>
  <form class="inner" onsubmit={create} novalidate>
    <h2 id="{uid}-title" class="title">New source</h2>

    <div class="kinds" role="radiogroup" aria-label="Kind">
      {#each KINDS as k (k.value)}
        <label class="kind vf vf-within" data-selected={kind === k.value ? "" : undefined}>
          <input type="radio" name="{uid}-kind" value={k.value} bind:group={kind} disabled={creating} />
          <span class="kind-text">
            <span class="kind-label">{k.label}</span>
            <span class="kind-desc">{k.desc}</span>
          </span>
        </label>
      {/each}
    </div>

    <div class="field">
      <label for="{uid}-name">Name</label>
      <div class="input vf vf-within">
        <input
          id="{uid}-name"
          type="text"
          bind:this={nameInput}
          bind:value={name}
          oninput={() => (nameTouched = true)}
          maxlength={SOURCE_NAME_MAX}
          required
          autocomplete="off"
          spellcheck="false"
          disabled={creating}
        />
      </div>
    </div>

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
    width: min(640px, calc(100vw - 2 * var(--s-6)));
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
  .kinds {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--s-2);
  }
  .kind {
    --vf-radius: var(--r-control);
    display: flex;
    align-items: flex-start;
    gap: var(--s-2);
    padding: var(--s-3);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
    cursor: pointer;
  }
  .kind[data-selected] {
    background: var(--selected-bg);
  }
  .kind input {
    margin-top: 2px;
    accent-color: var(--accent);
  }
  .kind-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .kind-label {
    color: var(--text);
    font-weight: 600;
  }
  .kind-desc {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    font-weight: 400;
  }
  .field {
    display: grid;
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

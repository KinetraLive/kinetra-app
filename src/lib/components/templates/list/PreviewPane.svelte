<!--
  Templates page side pane for the SELECTED template: the one live preview (LivePreview of the
  backend's `/screen/__preview`), then name (renamable in place), description, fields and actions.
  The page loads the template into the preview screen; this pane only shows it.

  Props:
    template: TemplateSummary
    origin: string                  "From Classic bar" / "Blank"
    fields: FieldDef[] | null       null until the full template has loaded
    src: string | null              preview screen URL; null while the server isn't running
    busy?: boolean                  a mutation is in flight (disables the actions)
    onrename: (name: string) => Promise<boolean>   resolves true when the rename was saved
    onduplicate: () => void
    ondelete: () => void
-->
<script lang="ts">
  import Copy from "lucide-svelte/icons/copy";
  import Pencil from "lucide-svelte/icons/pencil";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import Button from "$lib/components/Button.svelte";
  import LivePreview from "$lib/components/LivePreview.svelte";
  import type { FieldDef, TemplateSummary } from "$lib/types";
  import { NAME_MAX, validName } from "./format";

  type Props = {
    template: TemplateSummary;
    origin: string;
    fields: FieldDef[] | null;
    src: string | null;
    busy?: boolean;
    onrename: (name: string) => Promise<boolean>;
    onduplicate: () => void;
    ondelete: () => void;
  };
  let { template, origin, fields, src, busy = false, onrename, onduplicate, ondelete }: Props =
    $props();

  const uid = $props.id();
  let editing = $state(false);
  let draft = $state("");
  let saving = $state(false);
  let renameBtn: HTMLElement | undefined = $state();

  // Another selection ends an unfinished rename.
  $effect(() => {
    void template.id;
    editing = false;
  });

  function startRename() {
    draft = template.name;
    editing = true;
  }

  const focusSelect = (el: HTMLInputElement) => {
    el.focus();
    el.select();
  };

  function stop() {
    editing = false;
    queueMicrotask(() => renameBtn?.querySelector("button")?.focus());
  }

  async function commit(e?: SubmitEvent) {
    e?.preventDefault();
    if (saving || !editing) return;
    const name = draft.trim();
    if (name === template.name || !validName(name)) {
      stop();
      return;
    }
    saving = true;
    try {
      if (await onrename(name)) stop();
    } finally {
      saving = false;
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      stop();
    }
  }
</script>

<section class="pane" aria-labelledby="{uid}-label">
  <h2 id="{uid}-label" class="section-label">Preview</h2>
  <LivePreview {src} title="Template preview">
    {#if src === null}Server not running{/if}
  </LivePreview>

  <div class="details">
    {#if editing}
      <form class="rename" onsubmit={commit}>
        <label class="visually-hidden" for="{uid}-name">Template name</label>
        <div class="input vf vf-within">
          <input
            id="{uid}-name"
            type="text"
            bind:value={draft}
            {@attach focusSelect}
            maxlength={NAME_MAX}
            required
            autocomplete="off"
            spellcheck="false"
            disabled={saving}
            {onkeydown}
            onblur={() => void commit()}
          />
        </div>
      </form>
    {:else}
      <h3 class="name">{template.name}</h3>
    {/if}
    <p class="origin">{origin}</p>
    {#if template.presetUpdate}
      <!-- Only for an outdated preset template: its own small chunk. -->
      {#await import("../UpdateNotice.svelte") then { default: UpdateNotice }}
        <UpdateNotice target={template} />
      {/await}
    {/if}
    {#if template.description}<p class="desc">{template.description}</p>{/if}

    {#if fields && fields.length > 0}
      <h4 class="section-label sub">Fields</h4>
      <ul class="fields">
        {#each fields as f (f.key)}
          <li>
            <span class="label">{f.label}</span>
            <span class="type mono">{f.type}</span>
          </li>
        {/each}
      </ul>
    {/if}

    <div class="actions">
      <Button variant="primary" class="open" href="/templates/{template.id}">Open editor</Button>
      <Button onclick={onduplicate} disabled={busy}>
        {#snippet icon()}<Copy size={14} />{/snippet}
        Duplicate
      </Button>
      <span class="rename-btn" bind:this={renameBtn}>
        <Button onclick={startRename} disabled={busy || editing}>
          {#snippet icon()}<Pencil size={14} />{/snippet}
          Rename
        </Button>
      </span>
      <Button variant="danger" onclick={ondelete} disabled={busy}>
        {#snippet icon()}<Trash2 size={14} />{/snippet}
        Delete
      </Button>
    </div>
  </div>
</section>

<style>
  .pane {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    min-width: 0;
  }
  .details {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    min-width: 0;
    margin-top: var(--s-2);
  }
  .name {
    overflow-wrap: anywhere;
    font-size: var(--fs-lg);
  }
  .rename .input {
    --vf-radius: var(--r-control);
    display: flex;
    height: var(--control-h);
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .rename input {
    flex: 1;
    min-width: 0;
    padding: 0 var(--s-3);
    border: 0;
    background: transparent;
  }
  .origin,
  .desc {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .sub {
    margin-top: var(--s-3);
  }
  .fields {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .fields li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
    min-height: var(--row-h);
    border-bottom: 1px solid var(--line);
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .type {
    flex: none;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  /* The primary action gets its own full-width row; the three others share the next one in equal
     columns, so nothing wraps alone in the 400px pane. */
  .actions {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--s-2);
    margin-top: var(--s-4);
  }
  .actions :global(.open) {
    grid-column: 1 / -1;
  }
  .rename-btn {
    display: contents;
  }
</style>

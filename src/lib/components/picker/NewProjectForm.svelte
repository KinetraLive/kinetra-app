<!--
  Inline "New project" form for the picker. Name + parent folder, creates <parent>/<slug>.

  Props:
    id: string                        element id (the New project card's aria-controls target)
    suggestedLocation?: string | null prefilled parent folder (e.g. the last project's parent)
    oncreated: () => void             called after the project was created and opened
    oncancel: () => void              Esc: the picker collapses the form and restores focus

  Exposes `focusName()` for Ctrl+N.
-->
<script lang="ts">
  import { toAppError } from "$lib/api";
  import Button from "$lib/components/Button.svelte";
  import { pickFolder } from "$lib/projectFlow";
  import { project } from "$lib/stores/project.svelte";
  import { describeError, splitPath } from "./format";
  import { joinPath, slugify } from "./newProject";

  type Props = {
    id: string;
    suggestedLocation?: string | null;
    oncreated: () => void;
    oncancel: () => void;
  };

  let { id, suggestedLocation = null, oncreated, oncancel }: Props = $props();

  const NAME_MAX = 100;

  let name = $state("");
  let chosen = $state<string | null>(null);
  let picking = $state(false);
  let creating = $state(false);
  let error = $state<string | null>(null);
  let nameInput: HTMLInputElement | undefined = $state();

  const location = $derived(chosen ?? suggestedLocation ?? "");
  const locParts = $derived(splitPath(location));
  const trimmed = $derived(name.trim());
  const target = $derived(location && trimmed ? joinPath(location, slugify(trimmed)) : "");
  const canCreate = $derived(!!trimmed && !!location && !project.busy && !picking);

  export function focusName() {
    nameInput?.focus();
  }

  const autofocus = (el: HTMLInputElement) => el.focus();

  async function choose() {
    if (picking || project.busy) return;
    picking = true;
    try {
      const p = await pickFolder("Choose where to create the project");
      if (p) {
        chosen = p;
        error = null;
      }
    } catch (e) {
      error = describeError(toAppError(e));
    } finally {
      picking = false;
    }
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!canCreate) return;
    error = null;
    creating = true;
    try {
      await project.create(location, trimmed);
      oncreated();
    } catch (err) {
      error = describeError(toAppError(err));
    } finally {
      creating = false;
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      oncancel();
    }
  }
</script>

<!-- Esc anywhere inside the form collapses it; the handler only listens for bubbling key events. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<form {id} class="panel" aria-label="New project" onsubmit={submit} {onkeydown} novalidate>
  <div class="field">
    <label for="{id}-name">Name</label>
    <div class="input vf vf-within">
      <input
        id="{id}-name"
        type="text"
        bind:value={name}
        bind:this={nameInput}
        {@attach autofocus}
        maxlength={NAME_MAX}
        required
        autocomplete="off"
        spellcheck="false"
        aria-describedby="{id}-hint"
      />
    </div>
  </div>

  <div class="field">
    <span class="label" id="{id}-loc-label">Location</span>
    <div class="loc">
      {#if location}
        <output class="path mono" aria-labelledby="{id}-loc-label" title={location}
          ><span class="head">{locParts[0]}</span><span class="tail">{locParts[1]}</span></output
        >
      {/if}
      <Button onclick={choose} disabled={project.busy} loading={picking}>Choose…</Button>
    </div>
  </div>

  <div class="foot">
    <p class="hint" id="{id}-hint">
      {#if target}Creates <span class="mono target">{target}</span>{:else}Pick a name and a folder to put it in.{/if}
    </p>
    <Button variant="primary" type="submit" disabled={!canCreate && !creating} loading={creating}>
      Create
    </Button>
  </div>

  {#if error}<p class="error" role="alert">{error}</p>{/if}
</form>

<style>
  .panel {
    display: grid;
    gap: var(--s-3);
    padding: var(--s-4);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
    animation: reveal var(--dur) var(--ease);
  }
  .field {
    display: grid;
    gap: var(--s-1);
  }
  label,
  .label {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .input {
    --vf-radius: var(--r-control);
    display: flex;
    height: var(--control-h);
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  input {
    flex: 1;
    min-width: 0;
    padding: 0 var(--s-3);
    border: 0;
    background: transparent;
  }
  .loc {
    display: flex;
    gap: var(--s-2);
    min-width: 0;
  }
  .path {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    height: var(--control-h);
    padding: 0 var(--s-3);
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  /* Middle truncation: the folders shrink with an ellipsis, the last segment stays visible. */
  .head {
    min-width: 0;
    overflow: hidden;
    color: var(--text-muted);
    text-overflow: ellipsis;
  }
  .tail {
    flex: none;
    max-width: 70%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
    padding-top: var(--s-1);
  }
  .hint {
    min-width: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    overflow-wrap: anywhere;
  }
  .target {
    color: var(--text);
  }
  .error {
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  @keyframes reveal {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
</style>

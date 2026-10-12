<!--
  Recent projects list for the picker. Reads `project.recents`; rows open on click/Enter.
  Row actions (Show in Explorer, Remove from list) appear on hover and on keyboard focus.
  Missing folders show a warning tally dot + "Missing", are dimmed, not openable, and offer Remove.
  Keyboard focus on a row (and the row being opened) draws the viewfinder brackets around it.

  Props:
    onopened: () => void   called after a recent project opened successfully
    loadError?: string | null  shown instead of the empty line when the list failed to load
-->
<script lang="ts">
  import FolderSearch from "lucide-svelte/icons/folder-search";
  import X from "lucide-svelte/icons/x";
  import { toAppError } from "$lib/api";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import { revealInExplorer } from "$lib/projectFlow";
  import { project } from "$lib/stores/project.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { describeError, relativeTime, splitPath } from "./format";

  type Props = { onopened: () => void; loadError?: string | null };
  let { onopened, loadError = null }: Props = $props();

  /** Path of the row whose open is in flight / failed. */
  let opening = $state<string | null>(null);
  let failed = $state<{ path: string; message: string } | null>(null);

  // Computed once per list change; no timers.
  const rows = $derived.by(() => {
    const now = Date.now();
    return project.recents.map((r) => ({
      ...r,
      parts: splitPath(r.path),
      when: relativeTime(r.lastOpenedAt, now),
    }));
  });

  async function open(path: string) {
    if (project.busy) return;
    failed = null;
    opening = path;
    try {
      await project.open(path);
      onopened();
    } catch (e) {
      const message = describeError(toAppError(e));
      if (message) failed = { path, message };
    } finally {
      opening = null;
    }
  }

  async function reveal(path: string) {
    try {
      await revealInExplorer(path);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't show the folder: ${toAppError(e).message}` });
    }
  }

  async function remove(path: string) {
    try {
      await project.removeRecent(path);
      if (failed?.path === path) failed = null;
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't remove it: ${toAppError(e).message}` });
    }
  }
</script>

<section class="recents" aria-labelledby="recents-heading">
  <h2 id="recents-heading" class="section-label">Recent projects</h2>

  {#if rows.length === 0}
    <div class="panel">
      {#if loadError}
        <p class="note error" role="alert">{loadError}</p>
      {:else}
        <p class="note">Projects you open will appear here.</p>
      {/if}
    </div>
  {:else}
    <ul class="panel" aria-labelledby="recents-heading">
      {#each rows as r (r.path)}
        <li class="row vf" class:missing={r.missing} data-selected={opening === r.path || undefined}>
          {#if r.missing}
            <div class="main" title={r.path}>
              <span class="name">{r.name}</span>
              <span class="path mono"
                ><span class="head">{r.parts[0]}</span><span class="tail">{r.parts[1]}</span></span
              >
              <span class="when status"><TallyDot state="warning" />Missing</span>
            </div>
            <div class="actions shown">
              <button type="button" class="text-btn vf remove" onclick={() => remove(r.path)}>Remove</button>
            </div>
          {:else}
            <button
              type="button"
              class="main"
              title={r.path}
              aria-busy={opening === r.path || undefined}
              aria-describedby={failed?.path === r.path ? "recent-error" : undefined}
              onclick={() => open(r.path)}
            >
              <span class="name">{r.name}</span>
              <span class="path mono"
                ><span class="head">{r.parts[0]}</span><span class="tail">{r.parts[1]}</span></span
              >
              <span class="when">{opening === r.path ? "Opening…" : r.when}</span>
            </button>
            <div class="actions">
              <IconButton size="sm" tooltip="top" label="Show in Explorer" onclick={() => reveal(r.path)}>
                <FolderSearch size={14} />
              </IconButton>
              <IconButton variant="danger"
                size="sm"
                tooltip="top"
                label="Remove from list"
                class="remove"
                onclick={() => remove(r.path)}
              >
                <X size={14} />
              </IconButton>
            </div>
          {/if}
          {#if failed?.path === r.path}
            <p class="error" id="recent-error" role="alert">{failed.message}</p>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .recents {
    display: grid;
    gap: var(--s-2);
    min-width: 0;
  }
  h2 {
    padding: 0 var(--s-1);
  }
  .panel {
    list-style: none;
    margin: 0;
    padding: 0;
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  .note {
    padding: var(--s-3) var(--s-4);
    color: var(--text-muted);
  }
  .row {
    --vf-radius: var(--r-control);
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    border-radius: var(--r-control);
  }
  .row + .row {
    border-top: 1px solid var(--line);
  }
  .row:first-child {
    border-radius: var(--r-panel) var(--r-panel) var(--r-control) var(--r-control);
  }
  .row:last-child {
    border-radius: var(--r-control) var(--r-control) var(--r-panel) var(--r-panel);
  }
  .row:only-child {
    border-radius: var(--r-panel);
  }
  .row:not(.missing):hover {
    background: var(--surface-2);
  }
  /* Keyboard focus on the row's open button draws the focus ring around the whole row. */
  .main:focus-visible {
    outline: none;
  }
  .row:has(> .main:focus-visible)::after {
    inset: calc(-1 * var(--vf-gap));
    border: var(--vf-stroke) solid var(--focus);
    border-radius: calc(var(--vf-radius) + var(--vf-gap));
    opacity: 1;
  }
  /* The row opening: the selection border follows the row's own (per-corner) radius. */
  .row[data-selected] {
    background: var(--selected-bg);
  }
  .row[data-selected]:not(:has(> .main:focus-visible))::after {
    border-radius: inherit;
  }
  .main {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    grid-template-areas:
      "name when"
      "path when";
    column-gap: var(--s-4);
    min-width: 0;
    padding: var(--s-2) var(--s-2) var(--s-2) var(--s-4);
    text-align: left;
  }
  .name {
    grid-area: name;
    min-width: 0;
    overflow: hidden;
    font-size: var(--fs-md);
    font-weight: 500;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  /* Middle truncation: the folders shrink with an ellipsis, the last segment stays visible. */
  .path {
    grid-area: path;
    display: flex;
    min-width: 0;
    color: var(--text-muted);
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
  .head {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tail {
    flex: none;
    max-width: 70%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .when {
    grid-area: when;
    color: var(--text-muted);
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
  .status {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--warning);
    font-size: var(--fs-sm);
  }
  .actions {
    display: flex;
    gap: var(--s-1);
    padding-right: var(--s-3);
    opacity: 0;
    transition: opacity var(--dur-fast) var(--ease);
  }
  .row:hover .actions,
  .row:focus-within .actions,
  .actions.shown {
    opacity: 1;
  }
  .actions :global(.remove:hover:not(:disabled):not([aria-disabled="true"])) {
    color: var(--danger);
  }
  /* Looks like a ghost, small Button; a plain button keeps the Button component out of the
     picker's initial bundle (budget: 50 KB gz). */
  .text-btn {
    --vf-radius: var(--r-control);
    position: relative;
    display: inline-flex;
    align-items: center;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    color: var(--text-muted);
    font-family: var(--font-body);
    font-size: var(--fs-sm);
    font-weight: 500;
    line-height: 1;
    white-space: nowrap;
    cursor: pointer;
  }
  .text-btn:hover {
    background: var(--surface-3);
    color: var(--text);
  }
  .text-btn:active {
    background: var(--surface-2);
  }
  .missing .name,
  .missing .path {
    opacity: 0.6;
  }
  .error {
    grid-column: 1 / -1;
    padding: 0 var(--s-4) var(--s-2);
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  .note.error {
    padding: var(--s-3) var(--s-4);
  }
</style>

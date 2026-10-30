<!--
  AssetPicker: the project's images (assets/) in a native modal <dialog>. Lazy chunk: open it with
  `pickAsset()` from ./pick.ts, which mounts it on <body> and resolves with the choice.

    toolbar   search by name, "Import images…" (the system file dialog, multi-select; the
              imported image is selected)
    grid      a listbox of thumbnails (name, size in mono); the selected one carries the brackets
    footer    for the selected image: Rename, Delete (asks first, says how many screens and
              sources use it); then None, Cancel, Choose

  Thumbnails load from the running output server (`/assets/<name>`, lazy, fixed size); without a
  server only the names show. The list refreshes on `project-files-changed` (kind "asset").

  Keyboard: one tab stop in the grid; arrow keys / Home / End move the selection, Enter chooses.
  Double-click chooses. Escape (or a backdrop click) cancels.
-->
<script module lang="ts">
  export interface AssetPickerProps {
    /** The current value; its image starts selected. */
    current: string;
    label?: string;
    /** "/assets/<name>", "" for None, or null (cancelled). Called once. */
    onclose: (value: string | null) => void;
  }
</script>

<script lang="ts">
  import { backdropClose } from "$lib/components/backdrop";
  import { tick, untrack } from "svelte";
  import ImageIcon from "lucide-svelte/icons/image";
  import Pencil from "lucide-svelte/icons/pencil";
  import Search from "lucide-svelte/icons/search";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import Upload from "lucide-svelte/icons/upload";
  import { on, toAppError } from "$lib/api";
  import { deleteAsset, importAssets, listAssets, renameAsset } from "$lib/api/assets";
  import Button from "$lib/components/Button.svelte";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import { server } from "$lib/stores/server.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { ASSET_EXTENSIONS, ASSET_MAX_BYTES, type AssetInfo } from "$lib/types";
  import { assetName, filterAssets, formatBytes, isAssetValue, renameIssue, thumbUrl } from "./assets";

  let { current, label, onclose }: AssetPickerProps = $props();

  const uid = $props.id();
  let dialog: HTMLDialogElement | undefined = $state();
  let grid: HTMLDivElement | undefined = $state();
  let returnFocus: HTMLElement | null = null;
  let closed = false;

  let list = $state.raw<AssetInfo[] | null>(null);
  let loadError = $state<string | null>(null);
  let query = $state("");
  /** Selected asset name. */
  let selected = $state<string | null>(untrack(() => (isAssetValue(current) ? assetName(current) : null)));
  let importing = $state(false);

  // Rename (inline, for the selected image).
  let renaming = $state(false);
  let newName = $state("");
  let renameBusy = $state(false);
  let renameError = $state<string | null>(null);

  // Delete.
  let confirming = $state(false);
  let deleteBusy = $state(false);

  const shown = $derived(list ? filterAssets(list, query) : []);
  const chosen = $derived(list?.find((a) => a.name === selected) ?? null);
  /** The grid's one tab stop: the selected image when shown, else the first. */
  const tabStop = $derived(shown.some((a) => a.name === selected) ? selected : (shown[0]?.name ?? null));
  const renameProblem = $derived(
    renaming && chosen ? renameIssue(newName, chosen.name, (list ?? []).map((a) => a.name)) : null,
  );

  let gen = 0;
  async function refresh() {
    const g = ++gen;
    try {
      const next = await listAssets();
      if (g !== gen) return;
      list = next;
      loadError = null;
    } catch (e) {
      if (g === gen) loadError = toAppError(e).message;
    }
  }

  $effect(() => {
    const d = dialog;
    if (!d) return;
    returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (typeof d.showModal === "function") d.showModal();
    else d.setAttribute("open", "");
    void refresh().then(() => focusGrid());
    const off = on("project-files-changed", (p) => {
      if (p.kind === "asset") void refresh();
    });
    const unwatch = server.watch();
    return () => {
      off();
      unwatch();
      gen++;
    };
  });

  function close(value: string | null) {
    if (closed) return;
    closed = true;
    const d = dialog;
    if (d?.open) {
      if (typeof d.close === "function") d.close();
      else d.removeAttribute("open");
    }
    const el = returnFocus;
    returnFocus = null;
    if (el?.isConnected) el.focus();
    onclose(value);
  }

  function onCancelEvent(e: Event) {
    e.preventDefault();
    if (confirming || importing) return;
    close(null);
  }


  // ---- Selection --------------------------------------------------------------------------------

  async function focusGrid() {
    await tick();
    const el =
      grid?.querySelector<HTMLElement>('[role="option"][aria-selected="true"]') ??
      grid?.querySelector<HTMLElement>('[role="option"]');
    el?.focus();
  }

  function select(name: string) {
    if (renaming && name !== selected) renaming = false;
    selected = name;
  }

  async function onGridKey(e: KeyboardEvent) {
    if (shown.length === 0) return;
    const i = Math.max(
      0,
      shown.findIndex((a) => a.name === selected),
    );
    // Columns in the grid, for ArrowUp / ArrowDown.
    const first = grid?.querySelector<HTMLElement>('[role="option"]');
    const cols =
      first && grid ? Math.max(1, Math.floor(grid.clientWidth / Math.max(1, first.offsetWidth))) : 1;
    let next: number;
    switch (e.key) {
      case "ArrowLeft":
        next = i - 1;
        break;
      case "ArrowRight":
        next = i + 1;
        break;
      case "ArrowUp":
        next = i - cols;
        break;
      case "ArrowDown":
        next = i + cols;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = shown.length - 1;
        break;
      case "Enter":
        e.preventDefault();
        if (chosen) close(chosen.url);
        return;
      default:
        return;
    }
    e.preventDefault();
    next = Math.min(shown.length - 1, Math.max(0, next));
    select(shown[next].name);
    await focusGrid();
  }

  // ---- Import -----------------------------------------------------------------------------------

  async function importImages() {
    if (importing) return;
    importing = true;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        multiple: true,
        filters: [{ name: "Images", extensions: [...ASSET_EXTENSIONS] }],
      });
      const paths = picked === null ? [] : Array.isArray(picked) ? picked : [picked];
      if (paths.length === 0) return;
      const added = await importAssets(paths);
      await refresh();
      if (added.length > 0) {
        query = "";
        selected = added[0].name;
        await focusGrid();
      }
      toasts.push({
        kind: "info",
        message:
          added.length === 1
            ? `Imported “${added[0].name}”.`
            : `Imported ${added.length} images.`,
      });
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't import: ${toAppError(e).message}` });
    } finally {
      importing = false;
    }
  }

  // ---- Rename -----------------------------------------------------------------------------------

  async function startRename() {
    if (!chosen) return;
    newName = chosen.name;
    renameError = null;
    renaming = true;
    await tick();
    const input = dialog?.querySelector<HTMLInputElement>(`#${uid}-rename`);
    input?.focus();
    // Select the name without the extension.
    const dot = newName.lastIndexOf(".");
    input?.setSelectionRange(0, dot > 0 ? dot : newName.length);
  }

  async function commitRename() {
    if (!chosen || renameBusy) return;
    const name = newName.trim();
    if (name === chosen.name) {
      renaming = false;
      void focusGrid();
      return;
    }
    if (renameProblem) return;
    renameBusy = true;
    renameError = null;
    try {
      const info = await renameAsset(chosen.name, name);
      list = (list ?? []).map((a) => (a.name === chosen.name ? info : a));
      selected = info.name;
      renaming = false;
      void focusGrid();
    } catch (e) {
      renameError = toAppError(e).message;
    } finally {
      renameBusy = false;
    }
  }

  function onRenameKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      void commitRename();
    } else if (e.key === "Escape") {
      // Keeps the picker open: only the rename is cancelled.
      e.preventDefault();
      e.stopPropagation();
      renaming = false;
      void focusGrid();
    }
  }

  // ---- Delete -----------------------------------------------------------------------------------

  const deleteMessage = $derived.by(() => {
    if (!chosen) return "";
    const n = chosen.usedBy.length;
    const use =
      n === 0
        ? "No screen or source uses it."
        : `It is used by ${n} ${n === 1 ? "screen or source" : "screens and sources"}; those images will be empty.`;
    return `“${chosen.name}” will be deleted from the project's assets folder. ${use}`;
  });

  async function confirmDelete() {
    if (!chosen) return;
    const name = chosen.name;
    deleteBusy = true;
    try {
      await deleteAsset(name);
      const i = shown.findIndex((a) => a.name === name);
      list = (list ?? []).filter((a) => a.name !== name);
      const rest = shown.filter((a) => a.name !== name);
      selected = rest[Math.min(Math.max(0, i), rest.length - 1)]?.name ?? null;
      confirming = false;
      toasts.push({ kind: "info", message: `Deleted “${name}”.` });
      void focusGrid();
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    } finally {
      deleteBusy = false;
    }
  }

  const title = $derived(label ? `Choose image for ${label}` : "Choose image");
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialog}
  class="dialog"
  aria-labelledby="{uid}-title"
  oncancel={onCancelEvent}
  {@attach backdropClose(() => !confirming && close(null))}
>
  <div class="inner">
    <header class="head">
      <h2 id="{uid}-title" class="title">{title}</h2>
      <span class="search vf vf-within">
        <span class="search-icon" aria-hidden="true"><Search size={14} /></span>
        <input
          type="search"
          aria-label="Search images"
          autocomplete="off"
          spellcheck="false"
          bind:value={query}
        />
      </span>
      <Button size="sm" loading={importing} onclick={() => void importImages()}>
        {#snippet icon()}<Upload size={14} />{/snippet}
        Import images…
      </Button>
    </header>

    <div class="body">
      {#if loadError}
        <p class="bad" role="alert">Couldn't load the images: {loadError}</p>
      {:else if list === null}
        <p class="muted" role="status">Loading images…</p>
      {:else if list.length === 0}
        <div class="empty">
          <p>
            No images yet. Import PNG, JPG, SVG, GIF or WebP, up to
            <span class="mono">{formatBytes(ASSET_MAX_BYTES)}</span> each.
          </p>
          <Button variant="primary" size="sm" loading={importing} onclick={() => void importImages()}>
            {#snippet icon()}<Upload size={14} />{/snippet}
            Import images…
          </Button>
        </div>
      {:else if shown.length === 0}
        <p class="muted">No image name contains “{query.trim()}”.</p>
      {:else}
        <div
          class="grid"
          role="listbox"
          aria-label="Images"
          tabindex="-1"
          bind:this={grid}
          onkeydown={onGridKey}
        >
          {#each shown as a (a.name)}
            {@const isSel = a.name === selected}
            {@const src = thumbUrl(server.status, a.url)}
            <div
              class="tile vf"
              role="option"
              aria-selected={isSel}
              data-asset={a.name}
              tabindex={a.name === tabStop ? 0 : -1}
              onclick={() => select(a.name)}
              ondblclick={() => close(a.url)}
            >
              <span class="thumb" aria-hidden="true">
                {#if src}
                  <img {src} alt="" width="128" height="80" loading="lazy" decoding="async" />
                {:else}
                  <ImageIcon size={20} />
                {/if}
              </span>
              <span class="name" title={a.name}>{a.name}</span>
              <span class="meta mono">{formatBytes(a.bytes)}</span>
            </div>
          {/each}
        </div>
      {/if}
    </div>

    <footer class="foot">
      <span class="sel">
        {#if chosen && renaming}
          <span class="rename vf vf-within">
            <input
              id="{uid}-rename"
              class="mono"
              type="text"
              aria-label="New name for {chosen.name}"
              autocomplete="off"
              spellcheck="false"
              maxlength="140"
              bind:value={newName}
              aria-invalid={renameProblem || renameError ? "true" : undefined}
              aria-describedby={renameProblem || renameError ? `${uid}-rename-err` : undefined}
              disabled={renameBusy}
              onkeydown={onRenameKey}
            />
          </span>
          <Button size="sm" variant="primary" loading={renameBusy} disabled={!!renameProblem} onclick={() => void commitRename()}>
            Rename
          </Button>
          <Button size="sm" variant="ghost" onclick={() => (renaming = false)}>Cancel rename</Button>
          {#if renameProblem || renameError}
            <span class="bad small" id="{uid}-rename-err">{renameProblem ?? renameError}</span>
          {/if}
        {:else if chosen}
          <span class="mono small sel-name" title={chosen.name}>{chosen.name}</span>
          <Button size="sm" variant="ghost" onclick={() => void startRename()}>
            {#snippet icon()}<Pencil size={14} />{/snippet}
            Rename
          </Button>
          <Button size="sm" variant="danger" onclick={() => (confirming = true)}>
            {#snippet icon()}<Trash2 size={14} />{/snippet}
            Delete
          </Button>
        {/if}
      </span>
      <span class="acts">
        <Button size="sm" variant="ghost" onclick={() => close("")}>None</Button>
        <Button size="sm" onclick={() => close(null)}>Cancel</Button>
        <Button size="sm" variant="primary" disabled={!chosen} onclick={() => chosen && close(chosen.url)}>
          Choose
        </Button>
      </span>
    </footer>
  </div>
</dialog>

{#if confirming && chosen}
  <ConfirmDialog
    title="Delete image?"
    message={deleteMessage}
    confirmLabel="Delete"
    danger
    busy={deleteBusy}
    onconfirm={() => void confirmDelete()}
    oncancel={() => (confirming = false)}
  />
{/if}

<style>
  .dialog {
    width: min(880px, calc(100vw - 2 * var(--s-6)));
    max-height: calc(100vh - 2 * var(--s-6));
    margin: auto;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    color: var(--text);
    overflow: hidden;
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
    max-height: calc(100vh - 2 * var(--s-6));
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2) var(--s-3);
    padding: var(--s-4) var(--s-5);
    border-bottom: 1px solid var(--line);
  }
  .title {
    flex: 1 1 auto;
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .search {
    display: flex;
    position: relative;
    flex: 0 1 14rem;
    border-radius: var(--r-control);
  }
  /* A magnifier inside the field marks it as search (no placeholder text). */
  .search-icon {
    position: absolute;
    top: 50%;
    left: var(--s-2);
    display: inline-flex;
    color: var(--text-muted);
    transform: translateY(-50%);
    pointer-events: none;
    z-index: 1;
  }
  .search input,
  .rename input {
    width: 100%;
    min-width: 0;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .search input {
    padding-left: calc(var(--s-2) + 14px + var(--s-1));
  }
  .rename input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .body {
    flex: 1;
    min-height: 12rem;
    overflow: auto;
    padding: var(--s-4) var(--s-5);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(9rem, 1fr));
    gap: var(--s-3);
  }
  .grid:focus-visible {
    outline: none;
  }
  .tile {
    --vf-radius: var(--r-control);
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: var(--s-2);
    border-radius: var(--r-control);
    cursor: pointer;
    user-select: none;
  }
  .tile:hover {
    background: var(--surface-3);
  }
  .tile[aria-selected="true"] {
    background: var(--selected-bg);
  }
  .thumb {
    display: grid;
    place-items: center;
    height: 80px;
    overflow: hidden;
    border-radius: var(--r-control);
    background: var(--surface-1);
    color: var(--text-muted);
  }
  .thumb img {
    width: 100%;
    height: 80px;
    object-fit: contain;
  }
  .name {
    overflow: hidden;
    font-size: var(--fs-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-3);
    max-width: 52ch;
    color: var(--text-muted);
  }
  .foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-2);
    padding: var(--s-3) var(--s-5);
    border-top: 1px solid var(--line);
  }
  .sel,
  .acts {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .sel-name {
    max-width: 16rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rename {
    display: flex;
    width: 14rem;
    border-radius: var(--r-control);
  }
  .small {
    font-size: var(--fs-sm);
  }
  .muted {
    color: var(--text-muted);
  }
  .bad {
    color: var(--danger);
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

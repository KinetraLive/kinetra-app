<!--
  MediaLibrary: the project's media files (assets/: video, audio, images). A lazy chunk: the
  Sources page's Media tab ("manage") and the media picker dialog ("pick") mount it.

    toolbar   search by name, kind filter (All / Video / Audio / Images, or the picker's kinds),
              Grid / List, "Import…" (the system file dialog; copies with progress and Cancel)
    imports   one row per file being copied: name, bar, mono copied / total, Cancel per import
    items     grid (thumbnail, name, mono duration or size) or list (thumbnail, name, duration,
              resolution, codec, size); a severity icon on files with playback issues
    details   the selected file: preview, mono facts (duration, resolution, frame rate, codecs,
              alpha, size), "Used by", issues with severity icon, where (OBS, Mac…) and a Convert…
              disclosure with the ffmpeg command for this file and Copy; Rename, Delete

  Thumbnails: images from the running output server (lazy <img>); video posters drawn from the
  file's first frame once the tile is in view (poster.ts: one decode at a time, cached); audio an
  icon. Without a server only names and icons show. The list refreshes on `project-files-changed`
  (kind "asset") and when an import finishes (the imported file is selected).

  Keyboard: one tab stop in the items; arrows / Home / End move the selection, Enter chooses
  (pick mode).

  Props:
    mode?: "manage" | "pick"
    accept?: MediaKind[]            kinds shown (default all)
    initial?: string                the `/assets/<name>` value selected first
    onselected?: (a: AssetInfo | null) => void
    onchoose?: (a: AssetInfo) => void   double-click / Enter (pick mode)
-->
<script lang="ts">
  import { tick, untrack } from "svelte";
  import CircleAlert from "lucide-svelte/icons/circle-alert";
  import Copy from "lucide-svelte/icons/copy";
  import Film from "lucide-svelte/icons/film";
  import ImageIcon from "lucide-svelte/icons/image";
  import LayoutGrid from "lucide-svelte/icons/layout-grid";
  import List from "lucide-svelte/icons/list";
  import Music from "lucide-svelte/icons/music";
  import CircleX from "lucide-svelte/icons/circle-x";
  import Pencil from "lucide-svelte/icons/pencil";
  import Search from "lucide-svelte/icons/search";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import Upload from "lucide-svelte/icons/upload";
  import { on, toAppError } from "$lib/api";
  import { deleteAsset, listAssets, renameAsset } from "$lib/api/assets";
  import Button from "$lib/components/Button.svelte";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import Segmented from "$lib/components/control/Segmented.svelte";
  import { assetName, filterAssets, formatBytes, isAssetValue, renameIssue, thumbUrl } from "$lib/components/assets/assets";
  import { server } from "$lib/stores/server.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { ASSET_IMAGE_MAX_BYTES, ASSET_MEDIA_MAX_BYTES, type AssetInfo, type MediaKind } from "$lib/types";
  import { importer, jobBytes } from "./importer.svelte";
  import {
    PLATFORM_LABEL,
    codecText,
    formatTime,
    fpsText,
    kindOf,
    resolutionText,
    sortIssues,
    worstIssue,
  } from "./media";
  import { posterAttachment } from "./poster";

  const ALL: readonly MediaKind[] = ["video", "audio", "image"];

  let {
    mode = "manage",
    accept = ALL,
    initial = "",
    onselected = () => {},
    onchoose = () => {},
  }: {
    mode?: "manage" | "pick";
    accept?: readonly MediaKind[];
    initial?: string;
    onselected?: (a: AssetInfo | null) => void;
    onchoose?: (a: AssetInfo) => void;
  } = $props();

  const uid = $props.id();
  let items: HTMLDivElement | undefined = $state();

  let list = $state.raw<AssetInfo[] | null>(null);
  let loadError = $state<string | null>(null);
  let query = $state("");
  type Filter = "all" | MediaKind;
  let filter = $state<Filter>("all");
  let view = $state<"grid" | "list">("grid");
  let selected = $state<string | null>(untrack(() => (isAssetValue(initial) ? assetName(initial) : null)));

  const KIND_LABEL: Record<MediaKind, string> = { video: "Video", audio: "Audio", image: "Images" };
  const KIND_NOUN: Record<MediaKind, string> = { video: "Video", audio: "Audio", image: "Image" };
  const filterOptions = $derived<{ value: Filter; label: string }[]>([
    { value: "all", label: "All" },
    ...accept.map((k) => ({ value: k, label: KIND_LABEL[k] })),
  ]);

  const accepted = $derived((list ?? []).filter((a) => {
    const k = kindOf(a);
    return k !== null && accept.includes(k);
  }));
  const shown = $derived(filterAssets(accepted, query).filter((a) => filter === "all" || kindOf(a) === filter));
  const chosen = $derived(accepted.find((a) => a.name === selected) ?? null);
  const tabStop = $derived(shown.some((a) => a.name === selected) ? selected : (shown[0]?.name ?? null));
  $effect(() => onselected(chosen));

  // ---- Load ---------------------------------------------------------------------------------------

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
    void refresh();
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

  // An import finished: reload and select the first imported file.
  let seenFinished = untrack(() => importer.finished);
  $effect(() => {
    const n = importer.finished;
    if (n === seenFinished) return;
    seenFinished = n;
    const first = importer.lastImported[0];
    void refresh().then(() => {
      if (first && list?.some((a) => a.name === first)) {
        query = "";
        filter = "all";
        selected = first;
        void focusSelected();
      }
    });
  });

  // ---- Selection ----------------------------------------------------------------------------------

  async function focusSelected() {
    await tick();
    const el =
      items?.querySelector<HTMLElement>('[role="option"][aria-selected="true"]') ??
      items?.querySelector<HTMLElement>('[role="option"]');
    el?.focus();
  }

  function select(name: string) {
    if (renaming && name !== selected) renaming = false;
    selected = name;
  }

  async function onItemsKey(e: KeyboardEvent) {
    if (shown.length === 0) return;
    const i = Math.max(0, shown.findIndex((a) => a.name === selected));
    const first = items?.querySelector<HTMLElement>('[role="option"]');
    const cols = view === "grid" && first && items ? Math.max(1, Math.floor(items.clientWidth / Math.max(1, first.offsetWidth))) : 1;
    let next: number;
    switch (e.key) {
      case "ArrowLeft":
        next = view === "grid" ? i - 1 : i;
        break;
      case "ArrowRight":
        next = view === "grid" ? i + 1 : i;
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
        if (chosen && mode === "pick") onchoose(chosen);
        return;
      default:
        return;
    }
    e.preventDefault();
    next = Math.min(shown.length - 1, Math.max(0, next));
    select(shown[next].name);
    await focusSelected();
  }

  // ---- Rename / delete ----------------------------------------------------------------------------

  let renaming = $state(false);
  let newName = $state("");
  let renameBusy = $state(false);
  let renameError = $state<string | null>(null);
  const renameProblem = $derived(
    renaming && chosen ? renameIssue(newName, chosen.name, (list ?? []).map((a) => a.name), "file") : null,
  );

  async function startRename() {
    if (!chosen) return;
    newName = chosen.name;
    renameError = null;
    renaming = true;
    await tick();
    const input = document.getElementById(`${uid}-rename`) as HTMLInputElement | null;
    input?.focus();
    const dot = newName.lastIndexOf(".");
    input?.setSelectionRange(0, dot > 0 ? dot : newName.length);
  }

  async function commitRename() {
    if (!chosen || renameBusy) return;
    const name = newName.trim();
    if (name === chosen.name) {
      renaming = false;
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
      e.preventDefault();
      e.stopPropagation();
      renaming = false;
    }
  }

  let confirming = $state(false);
  let deleteBusy = $state(false);
  const deleteMessage = $derived.by(() => {
    if (!chosen) return "";
    const n = chosen.usedBy.length;
    const use =
      n === 0
        ? "No screen or source uses it."
        : `It is used by ${n} ${n === 1 ? "screen or source" : "screens and sources"}; those will be empty.`;
    return `“${chosen.name}” will be deleted from the project. ${use}`;
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
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    } finally {
      deleteBusy = false;
    }
  }

  // ---- Convert ------------------------------------------------------------------------------------

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toasts.push({ kind: "info", message: "Copied the command." });
    } catch {
      toasts.push({ kind: "error", message: "Couldn't copy. Select the command and copy it." });
    }
  }

  const importKinds = $derived(accept);
  const limitsText = $derived(
    accept.includes("video") || accept.includes("audio")
      ? `Video and audio up to ${ASSET_MEDIA_MAX_BYTES / 2 ** 30} GB, images up to ${ASSET_IMAGE_MAX_BYTES / 2 ** 20} MB.`
      : `Images up to ${ASSET_IMAGE_MAX_BYTES / 2 ** 20} MB.`,
  );
</script>

{#snippet kindIcon(k: MediaKind | null, size: number)}
  {#if k === "video"}<Film {size} />{:else if k === "audio"}<Music {size} />{:else}<ImageIcon {size} />{/if}
{/snippet}

{#snippet severity(s: "error" | "warning" | null, size: number)}
  {#if s === "error"}
    <span class="sev error" title="Won't play everywhere"><CircleX {size} aria-label="Won't play everywhere" /></span>
  {:else if s === "warning"}
    <span class="sev warning" title="Plays with a caveat"><CircleAlert {size} aria-label="Plays with a caveat" /></span>
  {/if}
{/snippet}

{#snippet thumb(a: AssetInfo, big = false)}
  {@const k = kindOf(a)}
  {@const src = thumbUrl(server.status, a.url)}
  <span class="thumb" class:big aria-hidden="true">
    {#if k === "image" && src}
      <img {src} alt="" loading="lazy" decoding="async" />
    {:else if k === "video" && src}
      {#key `${a.name}|${a.modifiedAt}`}
        <canvas {@attach posterAttachment(`${a.name}|${a.bytes}|${a.modifiedAt}`, src)}></canvas>
      {/key}
      <span class="icon over">{@render kindIcon(k, big ? 20 : 16)}</span>
    {:else}
      <span class="icon">{@render kindIcon(k, big ? 24 : 18)}</span>
    {/if}
  </span>
{/snippet}

<div class="library" class:pick={mode === "pick"}>
  <div class="toolbar">
    <span class="search vf vf-within">
      <span class="search-icon" aria-hidden="true"><Search size={14} /></span>
      <input type="search" aria-label="Search media" autocomplete="off" spellcheck="false" bind:value={query} />
    </span>
    {#if accept.length > 1}
      <Segmented label="Show" options={filterOptions} value={filter} onchange={(v) => (filter = v)} />
    {/if}
    <span class="spacer"></span>
    <span class="views" role="group" aria-label="View">
      <IconButton size="sm" label="Grid" pressed={view === "grid"} pressedFill={false} onclick={() => (view = "grid")}>
        <LayoutGrid size={14} />
      </IconButton>
      <IconButton size="sm" label="List" pressed={view === "list"} pressedFill={false} onclick={() => (view = "list")}>
        <List size={14} />
      </IconButton>
    </span>
    <Button size="sm" variant={mode === "manage" ? "primary" : undefined} onclick={() => void importer.pickAndImport(importKinds)}>
      {#snippet icon()}<Upload size={14} />{/snippet}
      Import…
    </Button>
  </div>

  {#if importer.jobs.length > 0}
    <section class="imports" aria-label="Importing">
      {#each importer.jobs as job (job.id)}
        {@const bytes = jobBytes(job)}
        <div class="job">
          <ul class="files">
            {#each job.names as name, i (i)}
              {@const f = job.files[i]}
              <li class="file">
                <span class="file-name" title={name}>{name}</span>
                {#if f?.error}
                  <span class="bad small">{f.error}</span>
                {:else}
                  <ProgressBar
                    class="bar"
                    label="Importing {name}"
                    value={f && f.total > 0 ? f.copied : undefined}
                    max={f?.total || 100}
                    valueText={f ? `${formatBytes(f.copied)} of ${formatBytes(f.total)}` : "Waiting"}
                  />
                  <span class="mono small muted tnum">{f ? `${formatBytes(f.copied)} / ${formatBytes(f.total)}` : "Waiting"}</span>
                {/if}
              </li>
            {/each}
          </ul>
          <span class="job-foot">
            <span class="mono small muted tnum">{formatBytes(bytes.copied)} / {formatBytes(bytes.total)}</span>
            <Button size="sm" variant="danger" disabled={job.cancelling} onclick={() => void importer.cancel(job.id)}>
              {job.cancelling ? "Canceling…" : "Cancel import"}
            </Button>
          </span>
        </div>
      {/each}
    </section>
  {/if}

  <div class="main">
    <div class="items-wrap">
      {#if loadError}
        <p class="bad pad" role="alert">Couldn't load the media: {loadError}</p>
      {:else if list === null}
        <p class="muted pad" role="status">Loading media…</p>
      {:else if accepted.length === 0}
        <div class="empty pad">
          <p>No media yet. {limitsText}</p>
          <Button variant="primary" size="sm" onclick={() => void importer.pickAndImport(importKinds)}>
            {#snippet icon()}<Upload size={14} />{/snippet}
            Import…
          </Button>
        </div>
      {:else if shown.length === 0}
        <p class="muted pad">No file matches “{query.trim()}”.</p>
      {:else}
        <div
          class="items {view}"
          role="listbox"
          aria-label="Media"
          tabindex="-1"
          bind:this={items}
          onkeydown={onItemsKey}
        >
          {#if view === "list"}
            <div class="row head" aria-hidden="true">
              <span></span><span>Name</span><span>Duration</span><span>Resolution</span><span>Codec</span><span>Size</span>
            </div>
          {/if}
          {#each shown as a (a.name)}
            {@const isSel = a.name === selected}
            {@const m = a.media}
            {@const worst = worstIssue(m?.issues)}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div
              class="item vf"
              class:row={view === "list"}
              class:tile={view === "grid"}
              role="option"
              aria-selected={isSel}
              data-asset={a.name}
              tabindex={a.name === tabStop ? 0 : -1}
              onclick={() => select(a.name)}
              ondblclick={() => mode === "pick" && onchoose(a)}
            >
              {@render thumb(a)}
              <span class="name" title={a.name}>{@render severity(worst, 12)}{a.name}</span>
              {#if view === "grid"}
                <span class="meta mono tnum">{m?.durationMs != null ? formatTime(m.durationMs) : formatBytes(a.bytes)}</span>
              {:else}
                <span class="mono tnum muted">{m?.durationMs != null ? formatTime(m.durationMs) : ""}</span>
                <span class="mono muted">{resolutionText(m) ?? ""}</span>
                <span class="mono muted codec">{codecText(m) ?? ""}</span>
                <span class="mono tnum muted">{formatBytes(a.bytes)}</span>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>

    <aside class="details" aria-label="Details">
      {#if chosen}
        {@const m = chosen.media}
        {@const k = kindOf(chosen)}
        {@render thumb(chosen, true)}
        {#if renaming}
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
              disabled={renameBusy}
              onkeydown={onRenameKey}
            />
          </span>
          <span class="acts">
            <Button size="sm" variant="primary" loading={renameBusy} disabled={!!renameProblem} onclick={() => void commitRename()}>Rename</Button>
            <Button size="sm" variant="ghost" onclick={() => (renaming = false)}>Cancel rename</Button>
          </span>
          {#if renameProblem || renameError}<span class="bad small">{renameProblem ?? renameError}</span>{/if}
        {:else}
          <h3 class="title mono" title={chosen.name}>{chosen.name}</h3>
        {/if}
        <dl class="facts">
          <dt>Type</dt>
          <dd>{k ? KIND_NOUN[k] : "File"}{m?.container ? ` · ${m.container.toUpperCase()}` : ""}</dd>
          {#if m?.durationMs != null}<dt>Duration</dt><dd class="mono tnum">{formatTime(m.durationMs, true)}</dd>{/if}
          {#if resolutionText(m)}<dt>Resolution</dt><dd class="mono">{resolutionText(m)}</dd>{/if}
          {#if fpsText(m)}<dt>Frame rate</dt><dd class="mono">{fpsText(m)}</dd>{/if}
          {#if m?.videoCodec}<dt>Video</dt><dd class="mono">{m.videoCodec}{m.alpha ? " · alpha" : ""}</dd>{/if}
          {#if m?.audioCodec}<dt>Audio</dt><dd class="mono">{m.audioCodec}</dd>{/if}
          <dt>Size</dt>
          <dd class="mono">{formatBytes(chosen.bytes)}</dd>
          <dt>Used by</dt>
          <dd>{chosen.usedBy.length === 0 ? "Nothing yet" : `${chosen.usedBy.length} ${chosen.usedBy.length === 1 ? "screen or source" : "screens and sources"}`}</dd>
        </dl>

        {#if m && m.issues.length > 0}
          <ul class="issues" aria-label="Playback issues">
            {#each sortIssues(m.issues) as issue, i (i)}
              <li class="issue {issue.severity}">
                <span class="issue-head">
                  {@render severity(issue.severity, 14)}
                  <span class="issue-text">
                    {issue.message}
                    <span class="where">{issue.platforms.map((p) => PLATFORM_LABEL[p]).join(", ")}</span>
                  </span>
                </span>
                {#if issue.convert}
                  <details class="convert">
                    <summary class="vf">Convert…</summary>
                    <p class="small">{issue.convert.label}</p>
                    <div class="cmd">
                      <code class="mono">{issue.convert.command}</code>
                      <IconButton size="sm" label="Copy command" tooltip="left" onclick={() => issue.convert && void copy(issue.convert.command)}>
                        <Copy size={14} />
                      </IconButton>
                    </div>
                    <p class="muted small">Run it with ffmpeg, then import the new file.</p>
                  </details>
                {/if}
              </li>
            {/each}
          </ul>
        {:else if m}
          <p class="muted small">Plays everywhere.</p>
        {/if}

        {#if !renaming}
          <span class="acts">
            <Button size="sm" variant="ghost" onclick={() => void startRename()}>
              {#snippet icon()}<Pencil size={14} />{/snippet}
              Rename
            </Button>
            <Button size="sm" variant="danger" onclick={() => (confirming = true)}>
              {#snippet icon()}<Trash2 size={14} />{/snippet}
              Delete
            </Button>
          </span>
        {/if}
      {:else if accepted.length > 0}
        <p class="muted small">Select a file to see its details.</p>
      {/if}
    </aside>
  </div>
</div>

{#if confirming && chosen}
  <ConfirmDialog
    title="Delete file?"
    message={deleteMessage}
    confirmLabel="Delete"
    danger
    busy={deleteBusy}
    onconfirm={() => void confirmDelete()}
    oncancel={() => (confirming = false)}
  />
{/if}

<style>
  .library {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2) var(--s-3);
    padding: var(--s-3) var(--s-4);
    border-bottom: 1px solid var(--line);
  }
  .spacer {
    flex: 1;
  }
  .views {
    display: inline-flex;
    gap: 2px;
  }
  .search {
    display: flex;
    position: relative;
    flex: 0 1 14rem;
    border-radius: var(--r-control);
  }
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
  .rename {
    display: flex;
    border-radius: var(--r-control);
  }
  .rename input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }

  /* ---- Imports ---- */
  .imports {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-3) var(--s-4);
    border-bottom: 1px solid var(--line);
    background: var(--surface-1);
  }
  .job {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }
  .files {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .file {
    display: grid;
    grid-template-columns: minmax(0, 14rem) minmax(6rem, 1fr) auto;
    align-items: center;
    gap: var(--s-3);
    font-size: var(--fs-sm);
  }
  .file-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .job-foot {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--s-3);
  }

  /* ---- Items ---- */
  .main {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(16rem, 20rem);
    flex: 1;
    min-height: 0;
  }
  .items-wrap {
    min-width: 0;
    min-height: 0;
    overflow: auto;
    padding: var(--s-3) var(--s-4);
  }
  .pad {
    padding: var(--s-2) 0;
  }
  .items:focus-visible {
    outline: none;
  }
  .items.grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(10rem, 1fr));
    gap: var(--s-3);
  }
  .items.list {
    display: flex;
    flex-direction: column;
  }
  .item {
    --vf-radius: var(--r-control);
    min-width: 0;
    border-radius: var(--r-control);
    cursor: pointer;
    user-select: none;
  }
  .item:hover {
    background: var(--surface-3);
  }
  .item[aria-selected="true"] {
    background: var(--selected-bg);
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--s-2);
  }
  .row {
    display: grid;
    grid-template-columns: 48px minmax(8rem, 2fr) 5rem 6rem minmax(6rem, 1.5fr) 5rem;
    align-items: center;
    gap: var(--s-3);
    min-height: var(--row-h);
    padding: 2px var(--s-2);
    font-size: var(--fs-sm);
  }
  .row.head {
    min-height: 0;
    padding-block: var(--s-1);
    color: var(--text-muted);
    font-size: var(--fs-xs);
    cursor: default;
  }
  .row .thumb {
    height: 27px;
  }
  .codec {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .thumb {
    position: relative;
    display: grid;
    place-items: center;
    height: 90px;
    overflow: hidden;
    border-radius: var(--r-control);
    background: var(--surface-1);
    color: var(--text-muted);
  }
  .thumb.big {
    height: 150px;
  }
  .thumb img,
  .thumb canvas {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .icon {
    display: inline-flex;
  }
  .icon.over {
    position: absolute;
    right: var(--s-1);
    bottom: var(--s-1);
    padding: 2px;
    border-radius: var(--r-control);
    background: var(--surface-1);
  }
  .name {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
    overflow: hidden;
    font-size: var(--fs-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .sev {
    display: inline-flex;
    flex: none;
  }
  .sev.error {
    color: var(--danger);
  }
  .sev.warning {
    color: var(--warning);
  }

  /* ---- Details ---- */
  .details {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    min-width: 0;
    min-height: 0;
    overflow: auto;
    padding: var(--s-3) var(--s-4);
    border-left: 1px solid var(--line);
    background: var(--surface-1);
  }
  .details > :global(*) {
    flex-shrink: 0;
  }
  .title {
    margin: 0;
    overflow: hidden;
    font-size: var(--fs-sm);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .facts {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: var(--s-1) var(--s-3);
    margin: 0;
    font-size: var(--fs-sm);
  }
  .facts dt {
    color: var(--text-muted);
  }
  .facts dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .issues {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .issue {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    padding: var(--s-2);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line);
    font-size: var(--fs-sm);
  }
  .issue-head {
    display: flex;
    gap: var(--s-2);
  }
  .issue-head .sev {
    margin-top: 2px;
  }
  .issue-text {
    display: flex;
    flex-direction: column;
  }
  .where {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .convert summary {
    --vf-radius: var(--r-control);
    width: fit-content;
    border-radius: var(--r-control);
    color: var(--accent-text);
    cursor: pointer;
  }
  .convert p {
    margin: var(--s-1) 0;
  }
  .cmd {
    display: flex;
    align-items: flex-start;
    gap: var(--s-1);
  }
  .cmd code {
    flex: 1;
    min-width: 0;
    padding: var(--s-1) var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-2);
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
    user-select: all;
  }
  .acts {
    display: inline-flex;
    flex-wrap: wrap;
    gap: var(--s-2);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-3);
    max-width: 52ch;
    color: var(--text-muted);
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
  .library.pick .main {
    grid-template-columns: minmax(0, 1fr) minmax(14rem, 17rem);
  }
  @container (max-width: 720px) {
    .main {
      grid-template-columns: minmax(0, 1fr);
    }
    .details {
      border-left: 0;
      border-top: 1px solid var(--line);
    }
  }
</style>

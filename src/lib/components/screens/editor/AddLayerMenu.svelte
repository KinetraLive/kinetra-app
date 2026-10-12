<!--
  AddLayerMenu: the "Add layer" button and its template picker (a menu of the project's templates
  plus "New template…", which opens the template gallery, and "Media…" (video, audio or an image
  from the media library) when `onmedia` is given).

  Keyboard: Enter/Space/ArrowDown open the menu on its first item; ArrowUp/ArrowDown/Home/End move;
  Escape closes and returns focus to the button; Tab closes. A press outside closes it (the window
  listener exists only while open).

  Props:
    templates: TemplateSummary[]
    disabled?: boolean       e.g. the layer limit is reached
    disabledReason?: string  shown as the button's title while disabled
    onpick: (templateId: string) => void
    onnewtemplate: () => void
    onmedia?: () => void     "Media…": a media layer (the caller opens the media picker)
-->
<script lang="ts">
  import { tick } from "svelte";
  import Film from "lucide-svelte/icons/film";
  import Plus from "lucide-svelte/icons/plus";
  import Button from "$lib/components/Button.svelte";
  import type { TemplateSummary } from "$lib/types";

  let {
    templates,
    disabled = false,
    disabledReason,
    onpick,
    onnewtemplate,
    onmedia,
  }: {
    templates: readonly TemplateSummary[];
    disabled?: boolean;
    disabledReason?: string;
    onpick: (templateId: string) => void;
    onnewtemplate: () => void;
    onmedia?: () => void;
  } = $props();

  const uid = $props.id();
  let open = $state(false);
  let root: HTMLDivElement | undefined = $state();
  let menu: HTMLDivElement | undefined = $state();

  const items = () => [...(menu?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? [])];

  async function show(focusLast = false) {
    if (disabled) return;
    open = true;
    await tick();
    const all = items();
    (focusLast ? all.at(-1) : all[0])?.focus();
  }

  function close(refocus: boolean) {
    open = false;
    if (refocus) root?.querySelector<HTMLButtonElement>(`#${CSS.escape(uid)}-btn`)?.focus();
  }

  function onButtonKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      void show();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      void show(true);
    }
  }

  function onMenuKey(e: KeyboardEvent) {
    const all = items();
    const i = all.indexOf(document.activeElement as HTMLButtonElement);
    let next = -1;
    if (e.key === "ArrowDown") next = (i + 1) % all.length;
    else if (e.key === "ArrowUp") next = (i - 1 + all.length) % all.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = all.length - 1;
    else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close(true);
      return;
    } else if (e.key === "Tab") {
      close(false);
      return;
    }
    if (next < 0) return;
    e.preventDefault();
    all[next]?.focus();
  }

  function pick(id: string) {
    close(true);
    onpick(id);
  }

  function newTemplate() {
    close(false);
    onnewtemplate();
  }

  function media() {
    close(false);
    onmedia?.();
  }

  $effect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (root && !root.contains(e.target as Node)) close(false);
    };
    window.addEventListener("pointerdown", onDown, true);
    return () => window.removeEventListener("pointerdown", onDown, true);
  });
</script>

<div class="add" bind:this={root}>
  <Button
    id="{uid}-btn"
    size="sm"
    {disabled}
    title={disabled ? disabledReason : undefined}
    aria-haspopup="menu"
    aria-expanded={open}
    aria-controls={open ? `${uid}-menu` : undefined}
    onclick={() => (open ? close(false) : void show())}
    onkeydown={onButtonKey}
  >
    {#snippet icon()}<Plus size={14} />{/snippet}
    Add layer
  </Button>

  {#if open}
    <!-- svelte-ignore a11y_interactive_supports_focus -->
    <div
      class="menu"
      id="{uid}-menu"
      role="menu"
      aria-label="Add a layer from a template"
      bind:this={menu}
      onkeydown={onMenuKey}
    >
      {#if templates.length > 0}
        <p class="group section-label" aria-hidden="true">Templates</p>
        {#each templates as t (t.id)}
          <button type="button" class="item vf" role="menuitem" tabindex="-1" onclick={() => pick(t.id)}>
            <span class="name">{t.name}</span>
            <span class="meta">{t.fieldCount} field{t.fieldCount === 1 ? "" : "s"}</span>
          </button>
        {/each}
        <div class="sep" role="separator"></div>
      {:else}
        <p class="empty">This project has no templates yet.</p>
      {/if}
      {#if onmedia}
        <button type="button" class="item vf" role="menuitem" tabindex="-1" onclick={media}>
          <span class="icon" aria-hidden="true"><Film size={14} /></span>
          <span class="name">Media…</span>
        </button>
      {/if}
      <button type="button" class="item vf" role="menuitem" tabindex="-1" onclick={newTemplate}>
        <span class="icon" aria-hidden="true"><Plus size={14} /></span>
        <span class="name">New template…</span>
      </button>
    </div>
  {/if}
</div>

<style>
  .add {
    position: relative;
  }
  .menu {
    position: absolute;
    z-index: 20;
    top: calc(100% + var(--s-1));
    left: 0;
    display: flex;
    flex-direction: column;
    width: max(100%, 16rem);
    max-height: 22rem;
    overflow: auto;
    padding: var(--s-1);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .group {
    padding: var(--s-1) var(--s-2);
  }
  .item {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: var(--row-h);
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    color: var(--text);
    text-align: left;
  }
  .item:hover,
  .item:focus-visible {
    background: var(--surface-2);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .icon {
    display: inline-flex;
    color: var(--accent-text);
  }
  .sep {
    height: 1px;
    margin: var(--s-1) 0;
    background: var(--line);
  }
  .empty {
    padding: var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
</style>

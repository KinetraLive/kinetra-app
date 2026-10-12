<!--
  ActionMenu: a menu button and its popup list for the Flows pages (row actions on /flows, "Connect
  to…" in the editor's inspector). The same behaviour as Control's Menu, kept apart so Control's
  route chunk keeps its Menu (a component both import is split out of it). `items` may be a
  function: it is called only while the menu is open (lists that cost something to build).

  Keyboard: Enter/Space/ArrowDown open on the first item, ArrowUp on the last; ArrowUp/ArrowDown/
  Home/End move; Escape closes and refocuses the button; Tab closes. A press outside closes it (the
  window listener exists only while open).

  Props:
    label: string          accessible name of the button (it should contain `text`) and of the menu
    text?: string          visible button text (omit for an icon-only button)
    icon?: Snippet
    items: ActionItem[] | (() => ActionItem[])
    empty?: string         shown when there are no items
    align?: "start" | "end"
    size?: "sm" | "md"
-->
<script lang="ts" module>
  export interface ActionItem {
    id: string;
    label: string;
    /** Mono detail on the right (a type, a time). */
    meta?: string;
    disabled?: boolean;
    onselect: () => void;
  }
</script>

<script lang="ts">
  import { tick, type Snippet } from "svelte";
  import ChevronDown from "lucide-svelte/icons/chevron-down";

  let {
    label,
    text,
    icon,
    items,
    empty,
    align = "start",
    size = "md",
  }: {
    label: string;
    text?: string;
    icon?: Snippet;
    items: readonly ActionItem[] | (() => readonly ActionItem[]);
    empty?: string;
    align?: "start" | "end";
    size?: "sm" | "md";
  } = $props();

  const uid = $props.id();
  let open = $state(false);
  let root: HTMLDivElement | undefined = $state();
  let button: HTMLButtonElement | undefined = $state();
  let menu: HTMLDivElement | undefined = $state();
  const shown = $derived(open ? (typeof items === "function" ? items() : items) : []);

  const all = () => [...(menu?.querySelectorAll<HTMLButtonElement>("[role=menuitem]:not(:disabled)") ?? [])];

  async function show(last = false) {
    open = true;
    await tick();
    const list = all();
    ((last ? list.at(-1) : list[0]) ?? menu)?.focus();
  }

  function close(refocus: boolean) {
    open = false;
    if (refocus) button?.focus();
  }

  function onButtonKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      void show(e.key === "ArrowUp");
    }
  }

  function onMenuKey(e: KeyboardEvent) {
    const list = all();
    const i = list.indexOf(document.activeElement as HTMLButtonElement);
    let next = -1;
    if (e.key === "ArrowDown") next = (i + 1) % list.length;
    else if (e.key === "ArrowUp") next = (i - 1 + list.length) % list.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = list.length - 1;
    else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close(true);
      return;
    } else if (e.key === "Tab") {
      close(false);
      return;
    }
    if (next < 0 || list.length === 0) return;
    e.preventDefault();
    list[next]?.focus();
  }

  function pick(item: ActionItem) {
    close(true);
    item.onselect();
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

<div class="wrap" bind:this={root}>
  <button
    bind:this={button}
    type="button"
    class="trigger vf {size}"
    class:icon-only={!text}
    aria-label={label}
    aria-haspopup="menu"
    aria-expanded={open}
    aria-controls={open ? `${uid}-menu` : undefined}
    onclick={() => (open ? close(false) : void show())}
    onkeydown={onButtonKey}
  >
    {#if icon}<span class="glyph" aria-hidden="true">{@render icon()}</span>{/if}
    {#if text}<span>{text}</span>{/if}
    <span class="glyph chev" aria-hidden="true"><ChevronDown size={14} /></span>
  </button>

  {#if open}
    <div
      class="menu"
      class:end={align === "end"}
      id="{uid}-menu"
      role="menu"
      tabindex="-1"
      aria-label={label}
      bind:this={menu}
      onkeydown={onMenuKey}
    >
      {#each shown as item (item.id)}
        <button type="button" class="item vf" role="menuitem" tabindex="-1" disabled={item.disabled} onclick={() => pick(item)}>
          <span class="name">{item.label}</span>
          {#if item.meta}<span class="meta mono">{item.meta}</span>{/if}
        </button>
      {:else}
        {#if empty}<p class="empty">{empty}</p>{/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
    display: inline-flex;
  }
  .trigger {
    --vf-radius: var(--r-control);
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    height: var(--control-h);
    padding: 0 var(--s-2) 0 var(--s-3);
    border-radius: var(--r-control);
    background: var(--surface-3);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
    font-size: var(--fs-md);
    font-weight: 500;
    white-space: nowrap;
  }
  .trigger.sm {
    height: var(--control-h-sm);
    font-size: var(--fs-sm);
  }
  .trigger.icon-only {
    padding: 0 var(--s-2);
  }
  .trigger:hover {
    background: color-mix(in srgb, var(--text) 6%, var(--surface-3));
  }
  .glyph {
    display: inline-flex;
  }
  .chev {
    color: var(--text-muted);
  }
  .menu {
    position: absolute;
    z-index: 20;
    top: calc(100% + var(--s-1));
    left: 0;
    display: flex;
    flex-direction: column;
    width: max-content;
    min-width: 14rem;
    max-width: 24rem;
    max-height: 22rem;
    overflow: auto;
    padding: var(--s-1);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    outline: none;
  }
  .menu.end {
    left: auto;
    right: 0;
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
  .item:hover:not(:disabled),
  .item:focus-visible {
    background: var(--surface-2);
  }
  .item:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta,
  .empty {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .empty {
    margin: 0;
    padding: var(--s-2);
  }
</style>

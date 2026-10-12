<!--
  Menu: a toolbar menu button and its popup list (plain items and checkbox items).

  Keyboard: Enter/Space/ArrowDown open on the first item, ArrowUp on the last; ArrowUp/ArrowDown/
  Home/End move; Escape closes and refocuses the button; Tab closes. A press outside closes it (the
  window listener exists only while open). Checkbox items keep the menu open.

  Props:
    label: string          accessible name of the button (and of the menu)
    text?: string          visible button text (omit for an icon-only button)
    icon?: Snippet         button icon
    items: MenuItem[]
    heading?: string       section label at the top of the menu
    align?: "start" | "end"   which edge of the button the menu lines up with (default "start")
    disabled?: boolean
-->
<script lang="ts" module>
  export interface MenuItem {
    id: string;
    label: string;
    /** A checkbox item (menuitemcheckbox) when set. */
    checked?: boolean;
    /** Detail on the right, e.g. a screen name or a resolution. */
    meta?: string;
    /** Show `meta` in the mono face (technical values only, e.g. a resolution). */
    metaMono?: boolean;
    disabled?: boolean;
    /** A destructive item (Delete…, Remove…): --danger text. */
    danger?: boolean;
    onselect: () => void;
  }
</script>

<script lang="ts">
  import { tick, type Snippet } from "svelte";
  import Check from "lucide-svelte/icons/check";
  import ChevronDown from "lucide-svelte/icons/chevron-down";

  let {
    label,
    text,
    icon,
    items,
    heading,
    align = "start",
    disabled = false,
  }: {
    label: string;
    text?: string;
    icon?: Snippet;
    items: readonly MenuItem[];
    heading?: string;
    align?: "start" | "end";
    disabled?: boolean;
  } = $props();

  const uid = $props.id();
  let open = $state(false);
  let root: HTMLDivElement | undefined = $state();
  let button: HTMLButtonElement | undefined = $state();
  let menu: HTMLDivElement | undefined = $state();

  const all = () => [...(menu?.querySelectorAll<HTMLButtonElement>("[role^=menuitem]:not(:disabled)") ?? [])];

  async function show(last = false) {
    if (disabled) return;
    open = true;
    await tick();
    const list = all();
    (last ? list.at(-1) : list[0])?.focus();
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
    else if (e.key === "Escape" && !e.shiftKey) {
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
    list[next]?.focus();
  }

  function pick(item: MenuItem) {
    if (item.checked === undefined) close(true);
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
    class="trigger vf"
    class:icon-only={!text}
    {disabled}
    aria-label={text ? undefined : label}
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
    <!-- svelte-ignore a11y_interactive_supports_focus -->
    <div
      class="menu"
      class:end={align === "end"}
      id="{uid}-menu"
      role="menu"
      aria-label={label}
      bind:this={menu}
      onkeydown={onMenuKey}
    >
      {#if heading}<p class="heading section-label" aria-hidden="true">{heading}</p>{/if}
      {#each items as item (item.id)}
        <button
          type="button"
          class="item vf"
          class:danger={item.danger}
          role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"}
          aria-checked={item.checked}
          tabindex="-1"
          disabled={item.disabled}
          onclick={() => pick(item)}
        >
          {#if item.checked !== undefined}
            <span class="box" aria-hidden="true">{#if item.checked}<Check size={12} />{/if}</span>
          {/if}
          <span class="name">{item.label}</span>
          {#if item.meta}<span class="meta" class:mono={item.metaMono}>{item.meta}</span>{/if}
        </button>
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
  .trigger.icon-only {
    padding: 0 var(--s-2);
  }
  .trigger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text) 6%, var(--surface-3));
  }
  .trigger:disabled {
    opacity: 0.5;
    cursor: default;
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
    max-width: 22rem;
    max-height: 22rem;
    overflow: auto;
    padding: var(--s-1);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .menu.end {
    left: auto;
    right: 0;
  }
  .heading {
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
  .item:hover:not(:disabled),
  .item:focus-visible {
    background: var(--surface-2);
  }
  .item.danger {
    color: var(--danger);
  }
  .item:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .box {
    display: inline-grid;
    place-items: center;
    flex: none;
    width: 16px;
    height: 16px;
    border-radius: 4px;
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--accent-text);
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
</style>

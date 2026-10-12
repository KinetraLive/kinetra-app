<!--
  App sidebar: section links, Preferences (opens the Preferences window) and the pin toggle at the
  bottom.

  Two modes, from the project's ui-state (`uiState.value.sidebarPinned`):
  - Rail (unpinned): a --rail-w icon column in the grid. Resting on it for HOVER_OPEN_MS (long
    enough that passing over it on the way to the content never opens it), or moving
    keyboard focus into it, opens an overlay panel (--sidebar-w, icon + label rows) that slides out
    over the content with transform + opacity, so the content never reflows. Keyboard focus moves
    to the matching overlay row and the rail goes inert meanwhile. The overlay closes
    HOVER_CLOSE_MS after the pointer leaves (when the pointer opened it), at once on a pointer
    press anywhere outside it, when focus leaves the sidebar, on Escape, or after following a link; focus inside it returns to the matching rail
    item.
  - Pinned: the labelled rows sit in the grid at --sidebar-w; no overlay.
  Labels live only in the expanded rows, never in pseudo-element tooltips, and the rail clips
  horizontally, so it can never grow a horizontal scrollbar.

  The active section (aria-current="page") is marked with accent text and a 6px accent dot in the
  rail's left gutter, at the same place in every mode. Brackets are for keyboard focus only (nav
  state is not a selectable object). The pin is a neutral icon button: aria-pressed carries the
  state, the Pin/PinOff icon shows it, no accent fill.

  Up/Down/Home/End move focus between the links; Tab works as usual. Timers exist only while the
  pointer is over the sidebar (or just leaving it); nothing runs at rest.
  No props.
-->
<script module lang="ts">
  /** Hover time before the overlay opens, and grace time before it closes after the pointer leaves. */
  export const HOVER_OPEN_MS = 400;
  export const HOVER_CLOSE_MS = 150;
</script>

<script lang="ts">
  import { tick } from "svelte";
  import { page } from "$app/state";
  import Pin from "lucide-svelte/icons/pin";
  import PinOff from "lucide-svelte/icons/pin-off";
  import Settings from "lucide-svelte/icons/settings";
  import IconButton from "$lib/components/IconButton.svelte";
  import { uiState } from "$lib/stores/uiState.svelte";
  import { keysFor } from "$lib/commands/bindings.svelte";
  import { ariaKeys } from "$lib/commands/keys";
  import { isAdvanced } from "$lib/stores/mode.svelte";
  import { NAV_ITEMS, SETTINGS_COMMAND, SETTINGS_HREF, isActive, showPreferences, toggleSidebarPin, type NavIcon } from "./nav";

  /** `command`: its keys (after the user's bindings) are the link's aria-keyshortcuts. */
  type Row = { idx: string; href: string; label: string; icon: NavIcon; command: string; advanced?: boolean };

  const ROWS: Row[] = NAV_ITEMS.map((item, i) => ({
    idx: String(i),
    href: item.href,
    label: item.label,
    icon: item.icon,
    command: item.id,
    advanced: item.advanced,
  }));
  /** Simple mode hides Flows. */
  const shown = $derived(isAdvanced() ? ROWS : ROWS.filter((r) => !r.advanced));
  const PREFS_ROW: Row = {
    idx: "prefs",
    href: SETTINGS_HREF,
    label: "Preferences",
    icon: Settings,
    command: SETTINGS_COMMAND,
  };

  const pinned = $derived(uiState.value?.sidebarPinned ?? false);
  const pathname = $derived(page.url.pathname);

  let rail: HTMLElement | undefined = $state();
  let overlay: HTMLElement | undefined = $state();
  let open = $state(false);
  /** The overlay is showing (never while pinned). */
  const expanded = $derived(open && !pinned);

  let openedBy: "pointer" | "focus" | null = null;
  let openTimer: ReturnType<typeof setTimeout> | undefined;
  let closeTimer: ReturnType<typeof setTimeout> | undefined;
  /** A pointer press is moving focus: don't treat that focus as keyboard entry. */
  let pointerFocus = false;
  /** We are moving focus back into the rail ourselves: don't reopen. */
  let restoring = false;

  function clearTimers() {
    clearTimeout(openTimer);
    clearTimeout(closeTimer);
    openTimer = closeTimer = undefined;
  }

  // Timers never outlive the component.
  $effect(() => clearTimers);

  let root: HTMLElement | undefined = $state();
  // While the overlay is open (only then), a press outside the sidebar collapses it at once, so it
  // never stays over the content.
  $effect(() => {
    if (!expanded) return;
    const onDown = (e: PointerEvent) => {
      if (e.target instanceof Node && root?.contains(e.target)) return;
      void hide(false);
    };
    document.addEventListener("pointerdown", onDown, true);
    return () => document.removeEventListener("pointerdown", onDown, true);
  });

  function show(by: "pointer" | "focus") {
    clearTimers();
    if (pinned || open) return;
    open = true;
    openedBy = by;
  }

  async function hide(restoreFocus = true) {
    clearTimers();
    if (!open) return;
    const active = document.activeElement;
    const idx =
      restoreFocus && overlay && active instanceof HTMLElement && overlay.contains(active)
        ? active.dataset.idx
        : undefined;
    open = false;
    openedBy = null;
    if (idx === undefined) return;
    restoring = true;
    try {
      await tick();
      rail?.querySelector<HTMLElement>(`[data-idx="${idx}"]`)?.focus({ preventScroll: true });
    } finally {
      restoring = false;
    }
  }

  function onmouseenter() {
    if (pinned) return;
    clearTimeout(closeTimer);
    closeTimer = undefined;
    if (open || openTimer) return;
    openTimer = setTimeout(() => {
      openTimer = undefined;
      show("pointer");
    }, HOVER_OPEN_MS);
  }

  function onmouseleave() {
    clearTimeout(openTimer);
    openTimer = undefined;
    if (!open || openedBy !== "pointer") return;
    closeTimer = setTimeout(() => {
      closeTimer = undefined;
      void hide();
    }, HOVER_CLOSE_MS);
  }

  async function onfocusin(e: FocusEvent) {
    const fromPointer = pointerFocus;
    pointerFocus = false;
    if (pinned || open || restoring || fromPointer) return;
    const target = e.target;
    if (!(target instanceof HTMLElement) || !rail?.contains(target)) return;
    show("focus");
    await tick();
    const idx = target.dataset.idx;
    if (idx !== undefined) {
      overlay?.querySelector<HTMLElement>(`[data-idx="${idx}"]`)?.focus({ preventScroll: true });
    }
  }

  function onfocusout(e: FocusEvent & { currentTarget: HTMLElement }) {
    if (!open) return;
    const to = e.relatedTarget;
    if (to instanceof Node && e.currentTarget.contains(to)) return;
    // Focus went nowhere (a click on plain content, or the window lost focus): a pointer-opened
    // overlay is left to the hover logic.
    if (to === null && openedBy === "pointer") return;
    void hide(false);
  }

  function onRootKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && open) {
      e.preventDefault();
      void hide();
    }
  }

  function onArrowKeydown(e: KeyboardEvent & { currentTarget: HTMLAnchorElement }) {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
    const root = e.currentTarget.closest("[data-navroot]");
    if (!root) return;
    const links = [...root.querySelectorAll<HTMLAnchorElement>("a[data-nav]")];
    const i = links.indexOf(e.currentTarget);
    e.preventDefault();
    const n = links.length;
    const next =
      e.key === "Home" ? 0 : e.key === "End" ? n - 1 : (i + (e.key === "ArrowDown" ? 1 : -1) + n) % n;
    links[next].focus();
  }

  function onLinkClick() {
    if (expanded) void hide();
  }

  /** Preferences is a window of its own: the link opens it instead of navigating. */
  function onPrefsClick(e: MouseEvent) {
    e.preventDefault();
    onLinkClick();
    showPreferences();
  }

  function onPinClick() {
    const active = document.activeElement;
    const fromOverlay = !!overlay && active instanceof Node && overlay.contains(active);
    clearTimers();
    open = false;
    openedBy = null;
    void toggleSidebarPin();
    if (fromOverlay) {
      void tick().then(() =>
        rail?.querySelector<HTMLElement>('[data-idx="pin"]')?.focus({ preventScroll: true }),
      );
    }
  }
</script>

{#snippet link(row: Row, labelled: boolean)}
  {@const active = isActive(pathname, row.href)}
  <a
    data-nav
    data-idx={row.idx}
    href={row.href}
    class="item vf"
    aria-current={active ? "page" : undefined}
    aria-keyshortcuts={keysFor(row.command).map(ariaKeys).join(" ") || undefined}
    aria-label={labelled ? undefined : row.label}
    onkeydown={onArrowKeydown}
    onclick={row === PREFS_ROW ? onPrefsClick : onLinkClick}
  >
    {#if active}<span class="current-dot" aria-hidden="true"></span>{/if}
    <span class="icon"><row.icon size={18} aria-hidden="true" /></span>
    {#if labelled}<span class="label">{row.label}</span>{/if}
  </a>
{/snippet}

{#snippet rows(labelled: boolean)}
  <ul class="list">
    {#each shown as row (row.idx)}
      <li>{@render link(row, labelled)}</li>
    {/each}
  </ul>
  <div class="foot">
    {@render link(PREFS_ROW, labelled)}
    <div class="pin">
      <IconButton
        label={pinned ? "Unpin sidebar" : "Pin sidebar"}
        pressed={pinned}
        pressedFill={false}
        tooltip="none"
        data-idx="pin"
        aria-keyshortcuts={keysFor("shell.pin").map(ariaKeys).join(" ") || undefined}
        onclick={onPinClick}
      >
        {#if pinned}<PinOff size={16} />{:else}<Pin size={16} />{/if}
      </IconButton>
    </div>
  </div>
{/snippet}

<!-- Hover/focus tracking for the overlay; the links and buttons inside are the interactive parts. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  bind:this={root}
  class="sidebar"
  data-mode={pinned ? "pinned" : "rail"}
  data-expanded={expanded ? "" : undefined}
  {onmouseenter}
  {onmouseleave}
  {onfocusin}
  {onfocusout}
  onkeydown={onRootKeydown}
  onpointerdown={() => (pointerFocus = true)}
  onpointerup={() => (pointerFocus = false)}
>
  <nav
    bind:this={rail}
    class="rail"
    class:labelled={pinned}
    aria-label="Main"
    data-navroot
    inert={expanded}
    aria-hidden={expanded ? "true" : undefined}
  >
    {@render rows(pinned)}
  </nav>

  {#if !pinned}
    <nav
      bind:this={overlay}
      class="overlay"
      class:open={expanded}
      aria-label="Main"
      aria-hidden={expanded ? undefined : "true"}
      data-navroot
      inert={!expanded}
    >
      {@render rows(true)}
    </nav>
  {/if}
</div>

<style>
  /* The grid cell. Overflow stays visible here so the overlay can extend over the content. */
  .sidebar {
    position: relative;
    z-index: 3;
    min-width: 0;
    min-height: 0;
  }

  .rail,
  .overlay {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-2);
    border-right: 1px solid var(--line);
  }

  .rail {
    height: 100%;
    background: var(--surface-1);
    /* Never a horizontal scrollbar: clip sideways; scroll vertically only in a tiny window.
       (overflow-x: clip next to overflow-y: auto computes to hidden anyway; neither shows a bar.) */
    overflow-x: hidden;
    overflow-y: auto;
  }

  .overlay {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
    width: var(--sidebar-w);
    background: var(--surface-2);
    overflow: hidden;
    /* Closed: tucked behind the rail, transparent, not painted, not hit-testable. */
    opacity: 0;
    visibility: hidden;
    pointer-events: none;
    transform: translateX(calc(var(--rail-w) - var(--sidebar-w)));
    transition:
      transform var(--dur) var(--ease),
      opacity var(--dur) var(--ease),
      visibility 0s linear var(--dur);
  }
  .overlay.open {
    opacity: 1;
    visibility: visible;
    pointer-events: auto;
    transform: none;
    transition:
      transform var(--dur) var(--ease),
      opacity var(--dur) var(--ease),
      visibility 0s;
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0;
    list-style: none;
  }

  .foot {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: auto;
    padding-top: var(--s-2);
    border-top: 1px solid var(--line);
  }

  /* A row: the icon sits in a --row-h square at the same x in every mode, so nothing jumps when
     the overlay opens or the sidebar is pinned. */
  .item {
    --vf-radius: var(--r-control);
    display: flex;
    align-items: center;
    width: var(--row-h);
    height: var(--row-h);
    border-radius: var(--r-control);
    color: var(--text-muted);
    font-weight: 500;
    text-decoration: none;
    white-space: nowrap;
  }
  .labelled .item,
  .overlay .item {
    width: 100%;
  }
  .icon {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: var(--row-h);
    height: var(--row-h);
  }
  .label {
    min-width: 0;
    padding-right: var(--s-2);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .item:hover {
    background: var(--surface-3);
    color: var(--text);
  }
  /* Active: accent text + a 6px accent dot in the rail's left gutter (no brackets: those are for
     focus and selected objects). Never accent text on surface-3 (contrast). */
  .item[aria-current="page"],
  .item[aria-current="page"]:hover {
    background: transparent;
    color: var(--accent-text);
  }
  .current-dot {
    position: absolute;
    top: calc(50% - 3px);
    /* Centred in the --s-2 gutter between the rail edge and the row. */
    left: calc(var(--s-2) / -2 - 3px);
    width: 6px;
    height: 6px;
    border-radius: var(--r-full);
    background: var(--accent);
    box-shadow: inset 0 0 0 1px var(--focus);
    pointer-events: none;
  }

  .pin {
    display: flex;
  }
</style>

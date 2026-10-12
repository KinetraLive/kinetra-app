<!--
  Popup: the layer / cue context menu and its Assign MIDI popover (mounted once on document.body by
  ./host.svelte.ts; reads its state). The menu: "Assign MIDI…" (the assigned source as detail),
  "Remove MIDI" when assigned, "Set hotkey"; in Simple mode only "Set hotkey" and the "Available in
  Advanced mode" hint (assigned controls keep working). Keyboard: focus starts on the first item;
  ArrowUp/ArrowDown/Home/End move; Enter/Space choose; Esc closes (focus back on the row); Tab and
  a press outside close. Placed at the point, kept inside the window.
-->
<script lang="ts">
  import { tick } from "svelte";
  import { controls } from "./controls.svelte";
  import { closePopup, popup, showAssign } from "./host.svelte";
  import { command, mappingsOf, removeTarget, sourceText } from "./model";
  import AssignPopover from "./AssignPopover.svelte";
  import { isAdvanced } from "$lib/stores/mode.svelte";

  const spec = $derived(popup.spec);
  const target = $derived.by(() => {
    const t = spec?.target;
    if (!t) return null;
    return command(t.kind === "layer" ? `live.toggle:${t.screenId}/${t.layerId}` : `cue.go:${t.flowId}`);
  });
  const assigned = $derived(target ? mappingsOf(controls.doc, target) : []);
  const advanced = $derived(isAdvanced());
  /** The menu items, and in Simple mode the hint's Switch button (arrow keys reach it too). */
  const FOCUSABLE = "[role=menuitem], .hint button";

  interface Item {
    id: string;
    label: string;
    meta?: string;
    danger?: boolean;
    run: () => void;
  }
  const items = $derived.by((): Item[] => {
    const s = spec;
    const t = target;
    if (!s || !t) return [];
    // Simple mode hides MIDI (assigned controls keep working; rows keep their MIDI chip).
    const out: Item[] = [];
    if (advanced) out.push({ id: "assign", label: "Assign MIDI…", meta: assigned[0] ? sourceText(assigned[0].source) : undefined, run: showAssign });
    if (advanced && assigned.length > 0)
      out.push({
        id: "remove",
        label: "Remove MIDI",
        danger: true,
        run: () => {
          closePopup();
          void controls.update((d) => removeTarget(d, t));
        },
      });
    if (s.setHotkey) {
      const set = s.setHotkey;
      out.push({
        id: "hotkey",
        label: "Set hotkey",
        run: () => {
          closePopup();
          set();
        },
      });
    }
    return out;
  });

  let box: HTMLElement | undefined = $state();

  /** Keeps the box inside the window, then focuses its first control. */
  function place(el: HTMLElement) {
    const r = el.getBoundingClientRect();
    const left = Math.max(8, Math.min(popup.x, innerWidth - r.width - 8));
    const top = popup.y + r.height > innerHeight - 8 ? Math.max(8, popup.y - r.height) : popup.y;
    el.style.left = `${left}px`;
    el.style.top = `${top}px`;
  }

  $effect(() => {
    void popup.mode;
    void popup.x;
    void popup.y;
    if (!spec || !box) return;
    const el = box;
    void tick().then(() => {
      place(el);
      if (popup.mode === "menu") el.querySelector<HTMLElement>(FOCUSABLE)?.focus();
    });
  });

  // A press outside closes (no refocus: the press moves focus itself).
  $effect(() => {
    if (!spec) return;
    const down = (e: PointerEvent) => {
      if (box && !box.contains(e.target as Node)) closePopup(false);
    };
    window.addEventListener("pointerdown", down, true);
    return () => window.removeEventListener("pointerdown", down, true);
  });

  function onkey(e: KeyboardEvent) {
    if (popup.mode !== "menu") return;
    const list = [...(box?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [])];
    const i = list.indexOf(document.activeElement as HTMLElement);
    let next = -1;
    if (e.key === "ArrowDown") next = (i + 1) % list.length;
    else if (e.key === "ArrowUp") next = (i - 1 + list.length) % list.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = list.length - 1;
    else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closePopup();
      return;
    } else if (e.key === "Tab") {
      closePopup(false);
      return;
    }
    if (next < 0) return;
    e.preventDefault();
    list[next]?.focus();
  }
</script>

{#if spec && target}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="popup" bind:this={box} style:left="{popup.x}px" style:top="{popup.y}px" onkeydown={onkey}>
    {#if popup.mode === "menu"}
      <div class="menu" role="menu" aria-label="Actions for {spec.name}">
        {#each items as it (it.id)}
          <button type="button" class="item vf" class:danger={it.danger} role="menuitem" tabindex="-1" onclick={it.run}>
            <span class="label">{it.label}</span>
            {#if it.meta}<span class="meta mono">{it.meta}</span>{/if}
          </button>
        {/each}
        {#if !advanced}
          {#await import("$lib/components/AdvancedHint.svelte") then { default: AdvancedHint }}
            <div class="hint"><AdvancedHint feature="MIDI" /></div>
          {/await}
        {/if}
      </div>
    {:else}
      <AssignPopover {target} name={spec.name} kind={spec.target.kind} onclose={() => closePopup()} />
    {/if}
  </div>
{/if}

<style>
  .popup {
    position: fixed;
    z-index: 60;
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .menu {
    display: flex;
    flex-direction: column;
    min-width: 13rem;
    max-width: 22rem;
    padding: var(--s-1);
  }
  .item {
    display: flex;
    align-items: center;
    gap: var(--s-3);
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
  .item.danger {
    color: var(--danger);
  }
  .label {
    flex: 1;
    white-space: nowrap;
  }
  .hint {
    padding: var(--s-1) var(--s-2);
  }
  .meta {
    color: var(--text-muted);
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
</style>

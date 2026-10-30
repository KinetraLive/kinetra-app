<!--
  MediaPicker: the media library in a native modal <dialog> for choosing a file (a lazy chunk:
  open it with `pickMedia()` from ./pick.ts, which mounts it on <body> and resolves with the
  choice). Footer: None (when `allowNone`), Cancel, Choose. Escape or a backdrop click cancels.
-->
<script module lang="ts">
  import type { AssetInfo, MediaKind } from "$lib/types";

  export interface MediaPickerProps {
    accept: readonly MediaKind[];
    current: string;
    title: string;
    allowNone?: boolean;
    /** The chosen file, `""` for None, or null (cancelled). Called once. */
    onclose: (value: AssetInfo | "" | null) => void;
  }
</script>

<script lang="ts">
  import { backdropClose } from "$lib/components/backdrop";
  import Button from "$lib/components/Button.svelte";
  import MediaLibrary from "./MediaLibrary.svelte";

  let { accept, current, title, allowNone = false, onclose }: MediaPickerProps = $props();

  const uid = $props.id();
  let dialog: HTMLDialogElement | undefined = $state();
  let returnFocus: HTMLElement | null = null;
  let closed = false;
  let chosen = $state.raw<AssetInfo | null>(null);

  $effect(() => {
    const d = dialog;
    if (!d) return;
    returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (typeof d.showModal === "function") d.showModal();
    else d.setAttribute("open", "");
  });

  function close(value: AssetInfo | "" | null) {
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
    close(null);
  }

</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog bind:this={dialog} class="dialog" aria-labelledby="{uid}-title" oncancel={onCancelEvent} {@attach backdropClose(() => close(null))}>
  <div class="inner">
    <header class="head">
      <h2 id="{uid}-title" class="title">{title}</h2>
    </header>
    <div class="body">
      <MediaLibrary mode="pick" {accept} initial={current} onselected={(a) => (chosen = a)} onchoose={(a) => close(a)} />
    </div>
    <footer class="foot">
      {#if allowNone}<Button size="sm" variant="ghost" onclick={() => close("")}>None</Button>{/if}
      <Button size="sm" onclick={() => close(null)}>Cancel</Button>
      <Button size="sm" variant="primary" disabled={!chosen} onclick={() => chosen && close(chosen)}>Choose</Button>
    </footer>
  </div>
</dialog>

<style>
  .dialog {
    width: min(1040px, calc(100vw - 2 * var(--s-6)));
    height: min(720px, calc(100vh - 2 * var(--s-6)));
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
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto;
    height: 100%;
  }
  .head {
    padding: var(--s-4) var(--s-5) var(--s-2);
  }
  .title {
    margin: 0;
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .body {
    min-height: 0;
    border-top: 1px solid var(--line);
  }
  .foot {
    display: flex;
    justify-content: flex-end;
    gap: var(--s-2);
    padding: var(--s-3) var(--s-5);
    border-top: 1px solid var(--line);
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

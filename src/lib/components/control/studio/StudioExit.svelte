<!--
  StudioExit: "Turn Studio mode off?" (lazy studio chunk), asked when a Preview still differs from
  Program. A native modal <dialog> (focus trap, Esc = Cancel, inert background).
    Take              the Previews go to Program first
    Keep for next take   cues are dropped; staged edits of on-air layers wait as "Changes on next
                      take", those of off-air layers apply
    Discard           the Previews are thrown away (destructive)
    Cancel            Studio mode stays on
  Initial focus is on Take.

  Props:
    screens: string[]                       names of the screens whose Preview differs
    onchoose: (c: StudioExit | null) => void
-->
<script lang="ts">
  import { backdropClose } from "$lib/components/backdrop";
  import Button from "$lib/components/Button.svelte";
  import type { StudioExit } from "./studio.svelte";

  let { screens, onchoose }: { screens: string[]; onchoose: (c: StudioExit | null) => void } = $props();

  const id = $props.id();
  let dialog: HTMLDialogElement | undefined = $state();

  $effect(() => {
    const d = dialog;
    if (!d || d.open) return;
    if (typeof d.showModal === "function") d.showModal();
    else d.setAttribute("open", "");
    d.querySelector<HTMLButtonElement>("[data-take]")?.focus();
  });

  const list = $derived(
    screens.length <= 1 ? (screens[0] ?? "a screen") : `${screens.slice(0, -1).join(", ")} and ${screens.at(-1)}`,
  );
  const many = $derived(screens.length > 1);

  function oncancel(e: Event) {
    e.preventDefault();
    onchoose(null);
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialog}
  class="dialog"
  aria-labelledby="{id}-title"
  aria-describedby="{id}-msg"
  {oncancel}
  {@attach backdropClose(() => onchoose(null))}
>
  <div class="inner">
    <h2 id="{id}-title" class="title">Turn Studio mode off?</h2>
    <p id="{id}-msg" class="msg">
      {many ? "The Previews of" : "The Preview of"} {list} {many ? "differ" : "differs"} from Program. Take to air
      first, keep the edits for the next take, or discard.
    </p>
    <ul class="what">
      <li>
        <strong>Keep for next take:</strong> on-air edits wait for the next take, the rest apply now. Cues are
        dropped.
      </li>
      <li><strong>Discard:</strong> Program stays as it is; the Preview is lost.</li>
    </ul>
    <div class="actions">
      <Button variant="ghost" onclick={() => onchoose(null)}>Cancel</Button>
      <span class="spacer"></span>
      <Button variant="danger" onclick={() => onchoose("discard")}>Discard</Button>
      <Button onclick={() => onchoose("keepStaged")}>Keep for next take</Button>
      <Button variant="accent" data-take onclick={() => onchoose("take")}>Take</Button>
    </div>
  </div>
</dialog>

<style>
  .dialog {
    width: min(560px, calc(100vw - 2 * var(--s-6)));
    max-height: calc(100vh - 2 * var(--s-6));
    margin: auto;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    color: var(--text);
    overflow: auto;
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
    gap: var(--s-2);
    padding: var(--s-5);
  }
  .title {
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .msg,
  .what {
    margin: 0;
    color: var(--text-muted);
  }
  .what {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    padding-left: var(--s-4);
    font-size: var(--fs-sm);
  }
  .what strong {
    color: var(--text);
    font-weight: 600;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
    margin-top: var(--s-4);
  }
  .spacer {
    flex: 1;
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

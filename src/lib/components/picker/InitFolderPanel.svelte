<!--
  "This folder isn't empty" confirmation shown under Open project when the chosen folder has
  unrelated files. Loaded on demand by the picker (it only appears after a folder dialog), so it
  stays out of the picker's initial bundle. Takes focus when it appears.

  Props: path, entries (item count), busy (another project operation runs), initializing,
  error (message or null), oninit(), oncancel().
-->
<script lang="ts">
  import Button from "$lib/components/Button.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { describeEntries } from "$lib/projectFlow";

  let {
    path,
    entries,
    busy = false,
    initializing = false,
    error = null,
    oninit,
    oncancel,
  }: {
    path: string;
    entries: number;
    busy?: boolean;
    initializing?: boolean;
    error?: string | null;
    oninit: () => void;
    oncancel: () => void;
  } = $props();

  const focusOnMount = (el: HTMLElement) => el.focus();
</script>

<section
  class="panel"
  aria-labelledby="confirm-title"
  aria-describedby="confirm-desc"
  tabindex="-1"
  {@attach focusOnMount}
>
  <h2 id="confirm-title"><TallyDot state="warning" />This folder isn't empty and isn't a Kinetra project</h2>
  <p id="confirm-desc">
    It has <span class="count">{describeEntries(entries)}</span>. Initializing adds the project
    files next to them and leaves everything else as it is.
  </p>
  <p class="mono path" title={path}>{path}</p>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <div class="actions">
    <Button variant="primary" loading={initializing} disabled={busy} onclick={oninit}>
      Initialize here anyway
    </Button>
    <Button variant="ghost" disabled={initializing} onclick={oncancel}>Cancel</Button>
  </div>
</section>

<style>
  .panel {
    display: grid;
    gap: var(--s-2);
    padding: var(--s-4);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--warning) 45%, var(--line));
    animation: reveal var(--dur) var(--ease);
  }
  .panel:focus-visible {
    outline: none;
  }
  h2 {
    display: flex;
    align-items: flex-start;
    gap: var(--s-2);
    font-size: var(--fs-md);
  }
  h2 :global(.tally) {
    margin-top: 4px;
  }
  p {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .count,
  .path {
    color: var(--text);
  }
  .path {
    overflow-wrap: anywhere;
  }
  .error {
    color: var(--danger);
  }
  .actions {
    display: flex;
    gap: var(--s-2);
    margin-top: var(--s-2);
  }

  @keyframes reveal {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
</style>

<!--
  Shown where Simple mode hides a feature: "Available in Advanced mode" with a one-click switch.
  `feature` names what is hidden ("MIDI and OSC"); omit it inside a section whose title already
  says so. Small and quiet: body text in --text-2, the switch a link-style button.
-->
<script lang="ts">
  import { t } from "$lib/i18n";
  import { setUiMode } from "$lib/stores/mode.svelte";

  let { feature }: { feature?: string } = $props();
</script>

<p class="hint">
  {feature ? t("{feature}: available in Advanced mode", { feature }) : t("Available in Advanced mode")}
  <button type="button" class="link" onclick={() => void setUiMode("advanced")}>{t("Switch to Advanced")}</button>
</p>

<style>
  .hint {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--s-2);
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm, 12px);
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
    border-radius: var(--r-control);
  }
  .link:hover {
    text-decoration: underline;
  }
  .link:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }
</style>

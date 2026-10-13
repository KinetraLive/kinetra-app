<!--
  A Pro feature: unlocked → its children; locked → a lock + "Pro" button that explains what Pro
  unlocks (a popover with Upgrade to Pro, or Sign in when signed out). Display only: the core
  refuses Pro commands on its own (`proRequired`).
  Props: feature ("ai" | "polls" | "control"), label (what the feature is called), children.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import Lock from "lucide-svelte/icons/lock";
  import Button from "$lib/components/Button.svelte";
  import { cloudLinks } from "$lib/api/license";
  import { openInBrowser } from "./open";
  import { license } from "./state.svelte";

  let { feature, label, children }: { feature: "ai" | "polls" | "control"; label: string; children: Snippet } = $props();

  license.start();
  const id = $props.id();
  const v = $derived(license.view);
  const unlocked = $derived(v?.features[feature] ?? false);
  const WHAT: Record<string, string> = {
    ai: "Describe a graphic and Kinetra designs the template for you, or tweak an existing one in plain words.",
    polls: "Run live polls: your audience votes on their phones and the results show on screen as they come in.",
    control: "Let your crew run graphics from a browser: take, clear, run cues and edit text from anywhere.",
  };
</script>

{#if unlocked}
  {@render children()}
{:else}
  <button type="button" class="lock vf" popovertarget="{id}-pro" aria-label="{label}: needs Kinetra Pro">
    <Lock size={12} aria-hidden="true" /> <span>{label}</span> <span class="pro">Pro</span>
  </button>
  <div id="{id}-pro" popover class="pop">
    <h3>{label} is part of Kinetra Pro</h3>
    <p>{WHAT[feature]}</p>
    <div class="row">
      {#if v && !v.signedIn}
        <Button variant="primary" onclick={() => void license.signIn()}>Sign in</Button>
      {/if}
      <Button onclick={() => void cloudLinks().then((l) => openInBrowser(l.pricing))}>Upgrade to Pro</Button>
    </div>
  </div>
{/if}

<style>
  .lock {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    height: var(--row-h);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text-muted);
    font: inherit;
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .lock:hover {
    color: var(--text);
  }
  .pro {
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--accent);
    color: var(--accent-text);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .pop {
    width: min(320px, calc(100vw - 32px));
    margin: auto;
    padding: var(--s-4);
    border: 0;
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text);
  }
  .pop::backdrop {
    background: color-mix(in srgb, var(--bg) 60%, transparent);
  }
  h3 {
    margin: 0 0 var(--s-2);
    font-family: var(--font-heading);
    font-size: var(--fs-md);
    font-weight: 600;
  }
  p {
    margin: 0 0 var(--s-3);
    color: var(--text-muted);
  }
  .row {
    display: flex;
    gap: var(--s-2);
  }
</style>

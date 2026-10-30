<!--
  First launch: until one sign-in or license key has succeeded on this computer, the app can't
  be used. "Sign in to get your free license" (primary) or "I have a license key"
  (KNTR-XXXX-XXXX-XXXX in any shape, tidied after a paste and on blur; redeemed after sign-in). Full-window; never shown
  again afterwards (signing out later doesn't bring it back).
-->
<script lang="ts">
  import BrandLogo from "$lib/components/brand/BrandLogo.svelte";
  import Button from "$lib/components/Button.svelte";
  import { signInCancel } from "$lib/api/license";
  import type { SigninView } from "$lib/types";
  import { normalizeKey, tidyKeyField } from "./messages";
  import SignInCode from "./SignInCode.svelte";
  import { license } from "./state.svelte";

  let { signin }: { signin: SigninView } = $props();

  let mode = $state<"choose" | "key">("choose");
  let key = $state("");
  const id = $props.id();
</script>

<div class="welcome" role="dialog" aria-modal="true" aria-labelledby="{id}-title">
  <div class="card">
    <div class="logo"><BrandLogo height={40} /></div>
    <h1 id="{id}-title">Welcome to Kinetra</h1>
    <p class="muted">Sign in once to start. Your free license is waiting.</p>
    {#if signin.state !== "idle"}
      <SignInCode
        {signin}
        onretry={() => void license.signIn()}
        oncancel={() => {
          license.pendingKey = null;
          void signInCancel();
        }}
      />
    {:else if mode === "key"}
      <form
        class="key"
        onsubmit={(e) => {
          e.preventDefault();
          const whole = normalizeKey(key);
          if (whole) void license.redeem((key = whole));
        }}
      >
        <label for="{id}-key">License key</label>
        <!-- svelte-ignore a11y_autofocus -->
        <input
          id="{id}-key"
          class="mono"
          autocomplete="off"
          spellcheck="false"
          autofocus
          placeholder="KNTR-XXXX-XXXX-XXXX"
          value={key}
          oninput={(e) => (key = tidyKeyField(e.currentTarget, e))}
          onblur={(e) => (key = tidyKeyField(e.currentTarget))}
        />
        <p class="muted">You'll sign in first; the key is added to your account.</p>
        <div class="row">
          <Button type="submit" variant="primary" disabled={!normalizeKey(key)}>Sign in and add key</Button>
          <Button variant="ghost" onclick={() => (mode = "choose")}>Back</Button>
        </div>
      </form>
    {:else}
      <div class="actions">
        <Button variant="primary" onclick={() => void license.signIn()}>Sign in to get your free license</Button>
        <Button onclick={() => (mode = "key")}>I have a license key</Button>
      </div>
    {/if}
  </div>
</div>

<style>
  .welcome {
    position: fixed;
    inset: 0;
    z-index: 900;
    display: grid;
    place-items: center;
    padding: var(--s-6);
    background: var(--bg);
  }
  .card {
    display: grid;
    gap: var(--s-4);
    width: min(440px, 100%);
    padding: var(--s-6);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  /* The artwork's left margin: line the bars up with the text. */
  .logo {
    margin-left: -5px;
    line-height: 0;
  }
  h1 {
    margin: 0;
    font-family: var(--font-heading);
    font-size: var(--fs-xl);
    font-weight: 600;
  }
  .actions,
  .key {
    display: grid;
    gap: var(--s-2);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
  }
  label {
    font-weight: 600;
  }
  input {
    height: var(--row-h);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text);
    font-size: var(--fs-md);
    letter-spacing: 0.04em;
  }
  input:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: 1px;
  }
  p {
    margin: 0;
  }
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
</style>

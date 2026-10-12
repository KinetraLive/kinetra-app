<!--
  Device-flow progress: the code to check in the browser (mono, large), "Open the browser again"
  and Cancel; or what went wrong (denied, expired, failed) with "Try again".
  Props: signin (SigninView, not idle), onretry, oncancel.
-->
<script lang="ts">
  import Button from "$lib/components/Button.svelte";
  import type { SigninView } from "$lib/types";
  import { openInBrowser } from "./open";

  let {
    signin,
    onretry,
    oncancel,
  }: { signin: Exclude<SigninView, { state: "idle" }>; onretry: () => void; oncancel: () => void } = $props();
</script>

{#if signin.state === "waiting"}
  <div class="code-box" role="status">
    <p>Approve the sign-in in your browser. Check that it shows this code:</p>
    <p class="code mono" aria-label="Code {signin.userCode.split('').join(' ')}">{signin.userCode}</p>
    <p class="muted">Waiting for approval…</p>
  </div>
  <div class="row">
    <Button onclick={() => void openInBrowser(signin.url)}>Open the browser again</Button>
    <Button variant="ghost" onclick={oncancel}>Cancel</Button>
  </div>
{:else}
  <p class="bad" role="alert">
    {#if signin.state === "denied"}Sign-in was denied.
    {:else if signin.state === "expired"}The sign-in code expired.
    {:else}{signin.message}{/if}
  </p>
  <div class="row">
    <Button variant="primary" onclick={onretry}>Try again</Button>
    <Button variant="ghost" onclick={oncancel}>Cancel</Button>
  </div>
{/if}

<style>
  .code-box {
    display: grid;
    gap: var(--s-2);
  }
  p {
    margin: 0;
  }
  .code {
    padding: var(--s-3) 0;
    font-size: 28px;
    letter-spacing: 0.08em;
  }
  .muted {
    color: var(--text-muted);
  }
  .bad {
    color: var(--danger);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
  }
</style>

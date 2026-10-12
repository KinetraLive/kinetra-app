<!--
  Everything licensing shows by itself (main window, mounted once on <body> by load.ts):
  - the first-launch Welcome screen;
  - sign-in progress started elsewhere (a modal with the code);
  - a quiet notice bar: reduced mode, a change waiting for the show to end, a wrong clock;
  - "Pro is active on <computer>. Move Pro here?" and the seats-full choices;
  - the Beta Ended screen (signed end message), only when nothing is on air: beta and canary
    builds stay blocked there ("Download the stable version"); stable builds can Continue (Free).
  Display only: the core decides and gates; nothing here sets license state.
-->
<script lang="ts">
  import Button from "$lib/components/Button.svelte";
  import { cloudLinks, licenseActivate, licenseDeactivate, licenseMoveHere, signInCancel } from "$lib/api/license";
  import { openInBrowser } from "./open";
  import AiJobsChip from "$lib/components/ai/AiJobsChip.svelte";
  import SignInCode from "./SignInCode.svelte";
  import Welcome from "./Welcome.svelte";
  import { license } from "./state.svelte";

  const v = $derived(license.view);

  /** The bar's text, or null. */
  const notice = $derived.by(() => {
    if (!v) return null;
    if (v.notice?.kind === "clockWrong") {
      const m = Math.abs(v.notice.minutes);
      const off = m >= 120 ? `${Math.round(m / 60)} hours` : `${m} min`;
      return `Your computer's date or time is wrong (${off} off). Fix it so Kinetra can check your license.`;
    }
    if (v.pending) {
      const why =
        v.status === "expired"
          ? "Your Pro subscription ended."
          : v.status === "revoked"
            ? "Your license was revoked. Contact hello@kinetra.live."
            : "Your license changed.";
      return `${why} Kinetra keeps running until you end the show.`;
    }
    if (v.status === "reduced") return "Kinetra needs to connect to check your license. Pro features are limited until it does.";
    if (v.status === "clockTampered") return "Your computer's clock is behind. Pro features return after Kinetra connects.";
    return null;
  });

  const blocked = $derived(v?.upgradeBlocked ?? null);
  const blockedKey = $derived(blocked ? blocked.activations.map((a) => a.id).join(",") : "");
  const showMove = $derived(!!blocked && blocked.activations.length > 0 && license.dismissed.movePro !== blockedKey);

  const seats = $derived(v?.notice?.kind === "seatsFull" ? v.notice : null);
  const seatsKey = $derived(seats ? seats.code + seats.activations.map((a) => a.id).join(",") : "");
  const showSeats = $derived(!!seats && license.dismissed.seats !== seatsKey);

  const beta = $derived(v?.beta?.ended && !v.onAir && !license.dismissed.beta ? v.beta : null);

  let busy = $state(false);
  async function act(label: string, f: () => Promise<void>) {
    busy = true;
    await license.run(label, f);
    busy = false;
  }
  const name = (n: string | null) => n || "another computer";
</script>

{#if v?.welcome}
  <Welcome signin={v.signin} />
{:else if v && v.signin.state !== "idle"}
  <div class="scrim" role="dialog" aria-modal="true" aria-label="Sign in to Kinetra">
    <div class="card">
      <h2>Sign in to Kinetra</h2>
      <SignInCode signin={v.signin} onretry={() => void license.signIn()} oncancel={() => void signInCancel()} />
    </div>
  </div>
{/if}

{#if beta}
  <div class="beta" role="dialog" aria-modal="true" aria-labelledby="beta-title">
    <div class="beta-body">
      {#each beta.endMessage as line, i (i)}
        {#if i === 0}<h1 id="beta-title">{line}</h1>{:else}<p>{line}</p>{/if}
      {/each}
      {#if beta.blocking}
        <Button variant="primary" onclick={() => void cloudLinks().then((l) => openInBrowser(l.download))}>Download the stable version</Button>
      {:else}
        <Button onclick={() => (license.dismissed.beta = true)}>Continue</Button>
      {/if}
    </div>
  </div>
{/if}

{#if showMove && blocked}
  <div class="scrim" role="dialog" aria-modal="true" aria-labelledby="move-title">
    <div class="card">
      {#if blocked.activations.length === 1}
        <h2 id="move-title">Pro is active on {name(blocked.activations[0].machine_name)}. Move Pro here?</h2>
        <p class="muted">Pro stops on that computer; it keeps the free license.</p>
        <div class="row">
          <Button variant="primary" disabled={busy} onclick={() => void act("Couldn't move Pro", () => licenseMoveHere(blocked.activations[0].id))}>
            Move Pro here
          </Button>
          <Button variant="ghost" onclick={() => (license.dismissed.movePro = blockedKey)}>Not now</Button>
        </div>
      {:else}
        <h2 id="move-title">Pro is in use on other computers. Move it here?</h2>
        <ul>
          {#each blocked.activations as a (a.id)}
            <li>
              <span>{name(a.machine_name)}</span>
              <Button size="sm" disabled={busy} onclick={() => void act("Couldn't move Pro", () => licenseMoveHere(a.id))}>Move Pro here</Button>
            </li>
          {/each}
        </ul>
        <div class="row"><Button variant="ghost" onclick={() => (license.dismissed.movePro = blockedKey)}>Not now</Button></div>
      {/if}
    </div>
  </div>
{/if}

{#if showSeats && seats}
  <div class="scrim" role="dialog" aria-modal="true" aria-labelledby="seats-title">
    <div class="card">
      {#if seats.code === "backup_in_use"}
        <h2 id="seats-title">This license already has a backup computer</h2>
      {:else}
        <h2 id="seats-title">All computers on this license are in use</h2>
      {/if}
      <ul>
        {#each seats.activations as a (a.id)}
          <li>
            <span>{name(a.machine_name)}{a.slot === "backup" ? " (backup)" : ""}</span>
            <Button
              size="sm"
              variant="danger"
              disabled={busy}
              onclick={() =>
                void act("Couldn't switch computers", async () => {
                  await licenseDeactivate(a.id);
                  await licenseActivate(seats.code === "backup_in_use" ? "backup" : "main");
                })}
            >
              Deactivate and use this computer
            </Button>
          </li>
        {/each}
      </ul>
      <div class="row">
        {#if seats.code === "seats_full"}
          <Button disabled={busy} onclick={() => void act("Couldn't use this computer as backup", () => licenseActivate("backup"))}>Use as backup</Button>
        {/if}
        <Button variant="ghost" onclick={() => (license.dismissed.seats = seatsKey)}>Not now</Button>
      </div>
    </div>
  </div>
{/if}

<AiJobsChip />

{#if notice && !v?.welcome}
  <div class="notice" role="status">{notice}</div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 950;
    display: grid;
    place-items: center;
    padding: var(--s-6);
    background: color-mix(in srgb, var(--bg) 70%, transparent);
  }
  .card {
    display: grid;
    gap: var(--s-3);
    width: min(460px, 100%);
    padding: var(--s-5);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  h2 {
    margin: 0;
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  p {
    margin: 0;
  }
  .muted {
    color: var(--text-muted);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
  }
  ul {
    display: grid;
    gap: var(--s-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
  }
  .beta {
    position: fixed;
    inset: 0;
    z-index: 980;
    display: grid;
    place-items: center;
    background: var(--bg);
  }
  .beta-body {
    display: grid;
    justify-items: center;
    gap: var(--s-3);
    text-align: center;
  }
  .beta h1 {
    margin: 0;
    font-family: var(--font-heading);
    font-size: var(--fs-title);
    font-weight: 600;
  }
  .beta p {
    color: var(--text-muted);
    font-size: var(--fs-lg);
  }
  .notice {
    position: fixed;
    bottom: var(--s-4);
    left: 50%;
    z-index: 800;
    max-width: min(640px, calc(100vw - 32px));
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text);
    font-size: var(--fs-sm);
    transform: translateX(-50%);
  }
</style>

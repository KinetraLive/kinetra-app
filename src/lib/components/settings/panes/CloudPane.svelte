<!--
  Preferences > Cloud backup (`cloud`): the account's cloud storage and this computer's backup
  settings. Pro (ProLock "cloud"); a lapsed account keeps the section to download its backups.
    Status            the quiet state text (§2.5) + Check now
    Storage           used of the plan's storage; uploaded today
    Back up new projects   per computer (on by default)
    During shows      Pause backups / Limit speed (+ kbit/s): nothing on air waits for a backup
  The core decides access and every value; this mirrors `cloud` (watched while mounted). No props.
-->
<svelte:options runes />

<script lang="ts">
  import { toAppError } from "$lib/api";
  import { cloudAnswer, cloudRefresh, cloudSetBackup } from "$lib/api/cloud";
  import Button from "$lib/components/Button.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import Select from "$lib/components/Select.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import { cloud } from "$lib/components/cloud/state.svelte";
  import { gb, questionWords, retentionWords, stateWords } from "$lib/components/cloud/words";
  import ProLock from "$lib/components/license/ProLock.svelte";
  import { license } from "$lib/components/license/state.svelte";
  import { cloudLinks } from "$lib/api/license";
  import { openInBrowser } from "$lib/components/license/open";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { CLOUD_LIMIT_KBPS, type CloudAnswer, type CloudOnAir, type CloudPrefs } from "$lib/types";

  $effect(() => cloud.watch());
  const view = $derived(cloud.view);
  const words = $derived(stateWords(view));
  const retention = $derived(retentionWords(view));
  const q = $derived(view?.quota ?? null);
  const project = $derived(view?.project ?? null);
  const question = $derived(project?.question ? questionWords(project.question) : null);

  let busy = $state(false);
  async function check(onDemand: boolean) {
    if (busy) return;
    busy = true;
    try {
      cloud.view = await cloudRefresh(onDemand);
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    } finally {
      busy = false;
    }
  }

  async function act(f: () => Promise<unknown>) {
    if (busy) return;
    busy = true;
    try {
      await f();
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    } finally {
      busy = false;
    }
  }
  const answer = (a: CloudAnswer) => act(async () => (cloud.view = await cloudAnswer(a)));
  const setBackup = (on: boolean) => act(async () => (cloud.view = await cloudSetBackup(on)));

  const DEFAULTS: CloudPrefs = { newProjects: true };
  async function save(patch: Partial<CloudPrefs>) {
    const current = prefs.value;
    if (!current) return;
    try {
      await prefs.update({ ...current, cloud: { ...DEFAULTS, ...current.cloud, ...patch } });
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't save the setting: ${toAppError(e).message}` });
    }
  }

  const ON_AIR: { value: CloudOnAir; label: string }[] = [
    { value: "pause", label: "Pause backups" },
    { value: "limit", label: "Limit speed" },
  ];
</script>

<SettingsSection title="Cloud backup">
  {#if view?.access === "available"}
    <SettingsRow label="Status">
      <p class="status" role="status">
        {#if words}
          <span class:warning={words.tone === "warning"}>{words.text}</span>
        {:else if view.state === "notInPlan"}
          <span>Cloud backup is part of Kinetra Pro.</span>
          <Button size="sm" onclick={() => void cloudLinks().then((l) => openInBrowser(l.pricing))}>Upgrade to Pro</Button>
        {:else if view.state}
          <span>Ready</span>
        {:else}
          <span>Not checked yet</span>
        {/if}
        <Button size="sm" disabled={busy} onclick={() => check(false)}>Check now</Button>
      </p>
    </SettingsRow>
    {#if q && view.state !== "notInPlan"}
      <SettingsRow label="Storage">
        <div class="usage">
          <ProgressBar
            value={Math.min(q.percent, 100)}
            label="Cloud storage used"
            valueText="{gb(q.usedBytes)} of {gb(q.limitBytes)}"
            tone={view.state === "full" || view.state === "warning" ? "warning" : "accent"}
          />
          <span class="mono">{gb(q.usedBytes)} of {gb(q.limitBytes)}</span>
        </div>
      </SettingsRow>
      <SettingsRow label="Uploaded today">
        <span class="mono">{gb(q.uploadTodayUsed)} of {gb(q.uploadTodayLimit)}</span>
      </SettingsRow>
      <SettingsRow label="Version history" description={retention ?? undefined}>
        <span class="tnum">{q.retentionDays} days</span>
      </SettingsRow>
    {/if}
    {#if project}
      <SettingsRow label="This project" description="Backs up while you work." for="pref-cloud-project">
        <Switch
          id="pref-cloud-project"
          label="Back up this project"
          hideLabel
          checked={project.backup}
          disabled={busy}
          onchange={setBackup}
        />
      </SettingsRow>
      {#if question}
        <SettingsRow label="This project's backup" stacked>
          <div class="question" role="status">
            <p>{question.text}</p>
            <div class="choices">
              {#each question.choices as c (c.answer)}
                <Button size="sm" variant={c.primary ? "primary" : undefined} disabled={busy} onclick={() => answer(c.answer)}>
                  {c.label}
                </Button>
              {/each}
            </div>
          </div>
        </SettingsRow>
      {:else if project.issue && project.backup}
        <SettingsRow label="This project's backup">
          <p class="status"><span class="warning">{project.issue}</span></p>
        </SettingsRow>
      {/if}
    {/if}
    <SettingsRow label="Back up new projects" for="pref-cloud-new">
      <Switch
        id="pref-cloud-new"
        label="Back up new projects"
        hideLabel
        checked={view.settings.newProjects}
        onchange={(on) => save({ newProjects: on })}
      />
    </SettingsRow>
    <SettingsRow label="During shows" description="Nothing on air waits for a backup." for="pref-cloud-onair">
      <div class="onair">
        <Select
          id="pref-cloud-onair"
          value={view.settings.onAir}
          options={ON_AIR}
          onchange={(v) => save({ onAir: v })}
        />
        {#if view.settings.onAir === "limit"}
          <NumberInput
            value={view.settings.limitKbps}
            min={CLOUD_LIMIT_KBPS.min}
            max={CLOUD_LIMIT_KBPS.max}
            step={100}
            digits={7}
            suffix="kbit/s"
            aria-label="Speed limit during shows"
            onchange={(v) => save({ limitKbps: v })}
          />
        {/if}
      </div>
    </SettingsRow>
  {:else if view?.access === "signedOut"}
    <SettingsRow label="Cloud backup" description="Sign in to back up your projects.">
      <Button size="sm" variant="primary" onclick={() => void license.signIn()}>Sign in</Button>
    </SettingsRow>
  {:else if view}
    <SettingsRow label="Cloud backup" description="Already backed up from another computer?">
      <div class="locked">
        <!-- Free: locked, so the children never show. -->
        <ProLock feature="cloud" label="Cloud backup">{" "}</ProLock>
        <Button size="sm" disabled={busy} onclick={() => check(true)}>Check for backups</Button>
      </div>
    </SettingsRow>
  {/if}
</SettingsSection>

<style>
  .status,
  .locked,
  .onair {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
    margin: 0;
    min-width: 0;
  }
  .warning {
    color: var(--warning);
  }
  .question p {
    margin: 0 0 var(--s-2);
  }
  .choices {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
  }
  .usage {
    display: grid;
    gap: var(--s-1);
    min-width: 0;
    max-width: 320px;
  }
</style>

<!--
  PollEditor: question, 2–8 answers, an optional time limit and "Show results to voters". Checked
  as you type with the core's rules (the server's message still wins). While a poll is open (or
  has votes) the question and answers are locked and say why; the toggle stays editable.

  Props:
    initial: Draft
    maxOptions?: number        the plan's limit (8 without one)
    textLocked?: string | null why the question and answers can't change (null: editable)
    durationLocked?: boolean   open polls keep their time limit
    submitLabel: string        "Create poll" / "Save changes"
    busy?: boolean
    dirtyOnly?: boolean        the button only when something changed (editing)
    onsubmit: (d: Draft) => void
-->
<script lang="ts">
  import { untrack } from "svelte";
  import Plus from "lucide-svelte/icons/plus";
  import X from "lucide-svelte/icons/x";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import { POLL_OPTION_MAX, POLL_OPTIONS_MAX, POLL_OPTIONS_MIN, POLL_QUESTION_MAX } from "$lib/types";
  import { checkDraft, formatDuration, parseDuration, type Draft } from "./model";

  let {
    initial,
    maxOptions = POLL_OPTIONS_MAX,
    textLocked = null,
    durationLocked = false,
    submitLabel,
    busy = false,
    dirtyOnly = false,
    onsubmit,
  }: {
    initial: Draft;
    maxOptions?: number;
    textLocked?: string | null;
    durationLocked?: boolean;
    submitLabel: string;
    busy?: boolean;
    dirtyOnly?: boolean;
    onsubmit: (d: Draft) => void;
  } = $props();

  const uid = $props.id();
  // Edits start from `initial`; different content (another poll, a save) starts over. Compared by
  // value: live vote updates re-create the poll object several times a second and must not wipe
  // what is being typed.
  const fresh = (): Draft => ({ ...initial, options: [...initial.options] });
  const freshDuration = () => formatDuration(initial.duration ?? 120);
  let draft = $state<Draft>(fresh());
  let durationText = $state(freshDuration());
  let touched = $state(false);
  const initialKey = $derived(JSON.stringify(initial));
  $effect.pre(() => {
    void initialKey;
    untrack(() => {
      draft = fresh();
      durationText = freshDuration();
      touched = false;
    });
  });

  const max = $derived(Math.min(Math.max(maxOptions, POLL_OPTIONS_MIN), POLL_OPTIONS_MAX));
  const problem = $derived(checkDraft(draft, max));
  const dirty = $derived(
    draft.question !== initial.question ||
      draft.showResults !== initial.showResults ||
      draft.duration !== initial.duration ||
      draft.options.length !== initial.options.length ||
      draft.options.some((o, i) => o !== initial.options[i]),
  );

  function setDuration(text: string) {
    durationText = text;
    const s = parseDuration(text);
    if (s !== null) draft.duration = s;
  }

  function submit(e: SubmitEvent) {
    e.preventDefault();
    touched = true;
    if (problem || busy) return;
    onsubmit({ ...draft, question: draft.question.trim(), options: draft.options.map((o) => o.trim()) });
  }
</script>

<form class="editor" onsubmit={submit} novalidate>
  <label class="field" for="{uid}-q">
    <span class="section-label">Question</span>
    <span class="input vf vf-within">
      <input
        id="{uid}-q"
        type="text"
        autocomplete="off"
        maxlength={POLL_QUESTION_MAX + 20}
        disabled={textLocked !== null}
        bind:value={draft.question}
      />
    </span>
  </label>

  <fieldset class="field" disabled={textLocked !== null}>
    <legend class="section-label">Answers</legend>
    <ol class="answers">
      {#each draft.options as _, i (i)}
        <li class="answer">
          <span class="n tnum" aria-hidden="true">{i + 1}</span>
          <span class="input vf vf-within">
            <input
              type="text"
              aria-label="Answer {i + 1}"
              autocomplete="off"
              maxlength={POLL_OPTION_MAX + 20}
              bind:value={draft.options[i]}
            />
          </span>
          <IconButton
            label="Remove answer {i + 1}"
            size="sm"
            disabled={textLocked !== null || draft.options.length <= POLL_OPTIONS_MIN}
            onclick={() => (draft.options = draft.options.filter((_, j) => j !== i))}
          >
            <X size={14} />
          </IconButton>
        </li>
      {/each}
    </ol>
    {#if textLocked === null && draft.options.length < max}
      <span>
        <Button size="sm" variant="ghost" onclick={() => (draft.options = [...draft.options, ""])}>
          {#snippet icon()}<Plus size={14} />{/snippet}
          Add answer
        </Button>
      </span>
    {/if}
    {#if textLocked}<p class="hint">{textLocked}</p>{/if}
  </fieldset>

  <div class="row">
    <Switch
      label="Time limit"
      checked={draft.duration !== null}
      disabled={durationLocked}
      onchange={(on) => (draft.duration = on ? (parseDuration(durationText) ?? 120) : null)}
    />
    {#if draft.duration !== null}
      <span class="input duration vf vf-within">
        <input
          class="mono"
          type="text"
          aria-label="Time limit (minutes:seconds)"
          autocomplete="off"
          disabled={durationLocked}
          value={durationText}
          oninput={(e) => setDuration(e.currentTarget.value)}
        />
      </span>
    {/if}
  </div>
  <div class="row">
    <Switch label="Show results to voters" bind:checked={draft.showResults} />
  </div>

  {#if touched && problem}<p class="bad" role="alert">{problem}</p>{/if}
  {#if !dirtyOnly || dirty}
    <div class="submit">
      <Button type="submit" variant="primary" loading={busy} disabled={touched && problem !== null}>{submitLabel}</Button>
    </div>
  {/if}
</form>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: var(--s-4);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }
  .answers {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .answer {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .n {
    width: 2ch;
    color: var(--text-muted);
    text-align: right;
  }
  .input {
    display: flex;
    flex: 1;
    min-width: 0;
    border-radius: var(--r-control);
  }
  .input input {
    width: 100%;
    min-width: 0;
    height: var(--control-h);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .input input:disabled {
    color: var(--text-muted);
  }
  .duration {
    flex: none;
    width: 10ch;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--s-3);
  }
  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .bad {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  .submit {
    display: flex;
  }
</style>

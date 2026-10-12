<!--
  Field: one labelled row of an inspector form. With `input`, renders a text input (text or
  date-time; mono for technical values) that reports every keystroke, or for `type: "time"` a
  TimeInput that reports a valid HH:mm on blur / Enter (an invalid time stays in the box, marked,
  and is never committed; `optional` allows empty); otherwise its children.

  Props:
    label: string
    input?: { value: string; type?: "text" | "time" | "datetime-local"; mono?: boolean;
              maxlength?: number; placeholder?: string; optional?: boolean }
    oninput?: (value: string) => void
    disabled?: boolean
    children?: Snippet
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import TimeInput from "$lib/components/inputs/TimeInput.svelte";

  let {
    label,
    input,
    oninput,
    disabled = false,
    children,
  }: {
    label: string;
    input?: {
      value: string;
      type?: "text" | "time" | "datetime-local";
      mono?: boolean;
      maxlength?: number;
      placeholder?: string;
      optional?: boolean;
    };
    oninput?: (value: string) => void;
    disabled?: boolean;
    children?: Snippet;
  } = $props();

  const uid = $props.id();
</script>

<div class="field">
  <label class="flabel" for={input ? `${uid}-in` : undefined}>{label}</label>
  {#if input?.type === "time"}
    <span class="tbox">
      <TimeInput
        id="{uid}-in"
        value={input.value}
        optional={input.optional}
        {disabled}
        onchange={(v) => oninput?.(v)}
      />
    </span>
  {:else if input}
    <span class="text vf vf-within">
      <input
        id="{uid}-in"
        type={input.type ?? "text"}
        class:mono={input.mono}
        value={input.value}
        maxlength={input.maxlength}
        placeholder={input.placeholder}
        autocomplete="off"
        spellcheck="false"
        {disabled}
        oninput={(e) => oninput?.(e.currentTarget.value)}
      />
    </span>
  {:else}
    <div class="ctl">{@render children?.()}</div>
  {/if}
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    min-width: 0;
  }
  .flabel {
    color: var(--text-muted);
    font-size: var(--fs-xs);
    font-weight: 500;
  }
  .ctl {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-1) var(--s-2);
    min-width: 0;
  }
  .ctl :global(.grow) {
    flex: 1 1 100%;
  }
  .tbox {
    display: flex;
    --time-bg: var(--surface-1);
  }
  .text {
    --vf-radius: var(--r-control);
    display: flex;
    height: var(--control-h-sm);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .text input {
    flex: 1;
    min-width: 0;
    padding: 0 var(--s-2);
    border: 0;
    background: transparent;
    color: var(--text);
  }
</style>

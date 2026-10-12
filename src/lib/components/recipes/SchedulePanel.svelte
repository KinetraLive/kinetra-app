<!--
  SchedulePanel: a schedule recipe's live panel in Control (lazy chunk). Two kinds:

  schedule   header: the current time (mono) and Undo. A big NEXT card (title, location,
             "starts in mm:ss" within the hour, else "at HH:MM", a DELAYED badge with +N min,
             large Start now / +5 min buttons) and what is NOW. Then the run of show: time (mono;
             the planned time struck through when moved), title, location, a status badge (TallyDot
             + text: NOW program, UP NEXT preview, DELAYED warning, PREVIOUS / CANCELLED muted,
             CANCELLED struck through). The NOW row is highlighted; the list scrolls to keep NOW
             and UP NEXT in view. Per item: Start / End and +5 on hover or focus, and a menu with
             every action (Start now, End, Set as next [optional "starts at"], Delay +5/+10/+15,
             Delay by…, Set start time…, Cancel / Restore, Back to automatic). Actions with an
             input open a small inline form under the row.
  countdown  the single target item (UP NEXT, else the first not over): a big countdown, its time,
             set target time, Start now, Reset (back to automatic), Undo.

  Keyboard (only while focus is inside the panel, never while typing): N = start UP NEXT,
  D = delay UP NEXT +5 min, Ctrl+Z = undo.

  Every action calls `scheduleAction`; its reply (a fresh view) replaces the state, and
  `schedule-changed` keeps it live (status changes at boundaries come from the server). Titles and
  overrides come from the schedule document (`getSchedule`), reloaded after an action and when the
  source's `updatedAt` changes.

  Time: the server's clock wins. The drift seen when a view arrives (`computedAt - Date.now()`)
  corrects the local clock. The clock ticks once per second only while this panel is on screen and
  the window is visible (clock.svelte.ts); nothing runs at rest.

  Props: SchedulePanelProps (./lazy.ts).
-->
<script lang="ts">
  import { tick } from "svelte";
  import Ellipsis from "lucide-svelte/icons/ellipsis";
  import Undo2 from "lucide-svelte/icons/undo-2";
  import { on, toAppError } from "$lib/api";
  import { scheduleAction, scheduleView } from "$lib/api/recipes";
  import { getSchedule } from "$lib/api/sources";
  import Badge from "$lib/components/Badge.svelte";
  import Button from "$lib/components/Button.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import TimeInput from "$lib/components/inputs/TimeInput.svelte";
  import type { MenuItem } from "$lib/components/control/Menu.svelte";
  import { isTyping } from "$lib/components/shell/nav";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { ScheduleAction, ScheduleDoc, ScheduleItem, ScheduleItemView, ScheduleView } from "$lib/types";
  import { clock, watchClockWhileVisible } from "./clock.svelte";
  import type { SchedulePanelProps } from "./lazy";
  import {
    DELAY_RANGE,
    STATUS_BADGE,
    atTime,
    clockText,
    countdownTarget,
    countdownText,
    dayBreaks,
    driftOf,
    hhmm,
    itemActions,
    lateMinutes,
    localDate,
    scheduleKey,
    startsText,
    validTime,
    type FormKind,
  } from "./schedule";

  let { sourceId, kind, name, updatedAt, Menu }: SchedulePanelProps = $props();

  const uid = $props.id();
  let root: HTMLElement | undefined = $state();
  let list: HTMLOListElement | undefined = $state();

  let view = $state.raw<ScheduleView | null>(null);
  let drift = $state(0);
  let doc = $state.raw<ScheduleDoc | null>(null);
  let failed = $state(false);
  /** The inline form under a row (or the countdown's target time). */
  let form = $state<{ itemId: string; kind: FormKind; time: string; minutes: number } | null>(null);
  /** Why the form's time can't be used (blocks Set), or null. */
  let timeInvalid = $state<string | null>(null);

  watchClockWhileVisible(() => root);

  const serverNow = $derived(clock.now + drift);
  const today = $derived(localDate(serverNow));
  const items = $derived(view?.items ?? []);
  const docItems = $derived(new Map<string, ScheduleItem>((doc?.items ?? []).map((i) => [i.id, i])));
  const nextItem = $derived(view?.nextId ? items.find((i) => i.id === view?.nextId) : undefined);
  const nowItem = $derived(view?.nowId ? items.find((i) => i.id === view?.nowId) : undefined);
  const target = $derived(view ? countdownTarget(view) : null);
  const breaks = $derived(dayBreaks(items));

  function apply(v: ScheduleView) {
    // An older view (a late reply) never replaces a newer one.
    if (view && v.computedAt < view.computedAt) return;
    view = v;
    drift = driftOf(v);
  }

  // The view: loaded per source, then kept live by `schedule-changed`.
  $effect(() => {
    const id = sourceId;
    let alive = true;
    view = null;
    failed = false;
    const off = on("schedule-changed", (v) => {
      if (alive && v.sourceId === id) apply(v);
    });
    scheduleView(id).then(
      (v) => alive && apply(v),
      () => alive && (failed = true),
    );
    return () => {
      alive = false;
      off();
    };
  });

  let docGen = 0;
  async function loadDoc(id: string) {
    const gen = ++docGen;
    try {
      const d = await getSchedule(id);
      if (gen === docGen) doc = d;
    } catch {
      // Titles stay as they were; the next change retries.
    }
  }

  // The document: per source and whenever it was saved.
  $effect(() => {
    void updatedAt;
    void loadDoc(sourceId);
  });

  const title = (id: string) => docItems.get(id)?.title || "Untitled";
  const locationOf = (id: string) => docItems.get(id)?.location ?? "";

  async function act(a: ScheduleAction) {
    try {
      apply(await scheduleAction(sourceId, a));
      form = null;
      void loadDoc(sourceId);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't change the schedule: ${toAppError(e).message}` });
    }
  }

  const undo = () => act({ do: "undo" });

  function onkeydown(e: KeyboardEvent) {
    const k = scheduleKey(e, isTyping(e.target));
    if (!k || !view) return;
    if (k === "undo") {
      if (!view.canUndo) return;
      e.preventDefault();
      e.stopPropagation();
      void undo();
      return;
    }
    const id = view.nextId;
    if (!id) return;
    e.preventDefault();
    void act(k === "startNext" ? { do: "startNow", itemId: id } : { do: "delay", itemId: id, minutes: 5 });
  }

  // ---- Inline forms -------------------------------------------------------------------------------

  async function openForm(v: ScheduleItemView, k: FormKind) {
    form = { itemId: v.id, kind: k, time: k === "next" ? "" : hhmm(v.start), minutes: 5 };
    timeInvalid = null;
    await tick();
    root?.querySelector<HTMLElement>("[data-form] input")?.focus();
  }

  function submitForm(e: SubmitEvent, v: ScheduleItemView) {
    e.preventDefault();
    const f = form;
    if (!f) return;
    if (f.kind !== "delay" && timeInvalid !== null) return;
    if (f.kind === "delay") {
      void act({ do: "delay", itemId: v.id, minutes: f.minutes });
    } else if (f.kind === "next") {
      if (f.time !== "" && !validTime(f.time)) return;
      void act({ do: "setNext", itemId: v.id, start: f.time === "" ? null : atTime(v.start, f.time) });
    } else {
      if (!validTime(f.time)) return;
      void act({ do: "setStart", itemId: v.id, start: atTime(v.start, f.time) });
    }
  }

  function onFormKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      form = null;
    }
  }

  function menuOf(v: ScheduleItemView): MenuItem[] {
    return itemActions(v, docItems.get(v.id), doc?.nextOverride ?? null).map((a) => ({
      id: a.id,
      label: a.label,
      onselect: () => {
        if (a.action) void act(a.action);
        else if (a.form) void openForm(v, a.form);
      },
    }));
  }

  // ---- Keep NOW and UP NEXT in view ----------------------------------------------------------------

  function reveal(id: string | null | undefined) {
    if (!list || !id) return;
    const el = [...list.querySelectorAll<HTMLElement>("[data-item]")].find((x) => x.dataset.item === id);
    if (!el) return;
    const top = el.offsetTop;
    const bottom = top + el.offsetHeight;
    if (top < list.scrollTop) list.scrollTop = top;
    else if (bottom > list.scrollTop + list.clientHeight) list.scrollTop = bottom - list.clientHeight;
  }

  $effect(() => {
    const next = view?.nextId;
    const now = view?.nowId;
    if (!list) return;
    void tick().then(() => {
      reveal(next);
      reveal(now);
    });
  });
</script>

{#snippet badge(v: ScheduleItemView, inNextCard = false)}
  {@const b = STATUS_BADGE[v.status]}
  {#if b && !(inNextCard && v.status === "next")}
    {@const late = v.status === "delayed" ? lateMinutes(v, docItems.get(v.id)) : 0}
    <Badge
      tally={b.tally}
      tone={v.status === "now" ? "onAir" : v.status === "delayed" ? "warning" : "neutral"}
      data-badge={v.status}
    >
      {b.text}{#if late > 0}&nbsp;<span class="mono">+{late} min</span>{/if}
    </Badge>
  {/if}
  <!-- The NEXT card is headed "Up next" already: no second UP NEXT badge there. -->
  {#if !inNextCard && v.id === view?.nextId && v.status !== "next"}
    <Badge tally="preview" data-badge="next">UP NEXT</Badge>
  {/if}
{/snippet}

{#snippet itemForm(v: ScheduleItemView)}
  {#if form && form.itemId === v.id}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <form class="iform" data-form={form.kind} onsubmit={(e) => submitForm(e, v)} onkeydown={onFormKey}>
      {#if form.kind === "delay"}
        <label for="{uid}-f">Delay {title(v.id)} by</label>
        <NumberInput
          id="{uid}-f"
          value={form.minutes}
          min={DELAY_RANGE.min}
          max={DELAY_RANGE.max}
          digits={4}
          suffix="min"
          onchange={(m) => form && (form.minutes = m)}
        />
        <Button size="sm" variant="primary" type="submit">Delay</Button>
      {:else}
        <label for="{uid}-f">{form.kind === "next" ? "Starts at (optional)" : "Start time"}</label>
        <TimeInput
          id="{uid}-f"
          value={form.time}
          optional={form.kind === "next"}
          onchange={(t) => form && (form.time = t)}
          oninvalid={(r) => (timeInvalid = r)}
        />
        <Button size="sm" variant="primary" type="submit" disabled={timeInvalid !== null}>
          {form.kind === "next" ? "Set as next" : "Set"}
        </Button>
      {/if}
      <Button size="sm" variant="ghost" onclick={() => (form = null)}>Cancel</Button>
    </form>
  {/if}
{/snippet}

<!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_no_noninteractive_tabindex -->
<section class="panel" aria-label="{name}: {kind === 'countdown' ? 'countdown' : 'schedule'}" tabindex="-1" bind:this={root} {onkeydown}>
  <div class="bar">
    <span class="clock mono" aria-label="Current time">{clockText(serverNow)}</span>
    <span class="spacer"></span>
    <Button size="sm" variant="ghost" disabled={!view?.canUndo} onclick={undo} aria-keyshortcuts="Control+Z">
      {#snippet icon()}<Undo2 size={14} />{/snippet}
      Undo
    </Button>
  </div>

  {#if failed && !view}
    <p class="muted pad">Couldn't load the schedule.</p>
  {:else if view && items.length === 0}
    <p class="muted pad">This schedule has no items yet. Add them on the Sources page.</p>
  {:else if view && kind === "countdown"}
    {#if target}
      <div class="card big-targets" data-countdown-target={target.id}>
        <div class="card-head">
          <span class="section-label">{target.status === "now" ? "Now" : "Counting down to"}</span>
          {@render badge(target)}
        </div>
        <div class="card-title">{title(target.id)}</div>
        <div class="count mono" data-countdown>
          {target.status === "now" ? "LIVE" : countdownText(target.startMs - serverNow)}
        </div>
        <div class="muted">at <span class="mono">{hhmm(target.start)}</span>{#if locationOf(target.id)} · {locationOf(target.id)}{/if}</div>
        {#if form && form.itemId === target.id}
          {@render itemForm(target)}
        {:else}
          <div class="actions">
            <Button variant="primary" onclick={() => act({ do: "startNow", itemId: target.id })}>Start now</Button>
            <Button onclick={() => openForm(target, "start")}>Set target time</Button>
            <Button variant="ghost" onclick={() => act({ do: "auto", itemId: target.id })}>Reset</Button>
          </div>
        {/if}
      </div>
    {/if}
  {:else if view}
    {#if nextItem}
      <div class="card big-targets" data-next={nextItem.id}>
        <div class="card-head">
          <span class="section-label">Up next</span>
          {@render badge(nextItem, true)}
        </div>
        <div class="card-title">{title(nextItem.id)}</div>
        <div class="card-meta">
          <span class="starts" data-starts>{startsText(nextItem, serverNow, today)}</span>
          {#if locationOf(nextItem.id)}<span class="muted">· {locationOf(nextItem.id)}</span>{/if}
        </div>
        <div class="actions">
          <Button variant="primary" aria-keyshortcuts="N" onclick={() => act({ do: "startNow", itemId: nextItem.id })}>
            Start now <kbd class="mono">N</kbd>
          </Button>
          <Button aria-keyshortcuts="D" onclick={() => act({ do: "delay", itemId: nextItem.id, minutes: 5 })}>
            +5 min <kbd class="mono">D</kbd>
          </Button>
        </div>
      </div>
    {:else}
      <p class="muted pad">Nothing is up next.</p>
    {/if}
    {#if nowItem}
      <div class="nowline">
        <Badge tally="program" tone="onAir">NOW</Badge>
        <span class="now-title">{title(nowItem.id)}</span>
        <span class="muted">until <span class="mono">{hhmm(nowItem.end)}</span></span>
      </div>
    {/if}

    <ol class="items" aria-label="Run of show" bind:this={list}>
      {#each items as v (v.id)}
        {@const it = docItems.get(v.id)}
        {@const day = breaks.get(v.id)}
        {#if day}<li class="day section-label" aria-hidden="true">{day}</li>{/if}
        <li
          class="item"
          class:is-now={v.status === "now"}
          class:is-past={v.status === "previous" || v.status === "cancelled"}
          data-item={v.id}
          data-status={v.status}
        >
          <div class="line">
            <span class="time mono">
              {#if v.moved && it && hhmm(it.start) !== hhmm(v.start)}<s class="was">{hhmm(it.start)}</s>{/if}
              {hhmm(v.start)}
            </span>
            <span class="text">
              <span class="title" class:struck={v.status === "cancelled"}>{title(v.id)}</span>
              {#if it?.location}<span class="loc">{it.location}</span>{/if}
            </span>
            <span class="badges">{@render badge(v)}</span>
            <span class="quick">
              {#if v.status === "now"}
                <Button size="sm" onclick={() => act({ do: "end", itemId: v.id })} aria-label="End {title(v.id)}">End</Button>
              {:else if v.status !== "cancelled"}
                <Button size="sm" onclick={() => act({ do: "startNow", itemId: v.id })} aria-label="Start {title(v.id)} now">
                  Start
                </Button>
              {/if}
              {#if v.status !== "cancelled" && v.status !== "previous"}
                <Button
                  size="sm"
                  onclick={() => act({ do: "delay", itemId: v.id, minutes: 5 })}
                  aria-label="Delay {title(v.id)} 5 minutes"
                >
                  +5
                </Button>
              {/if}
            </span>
            <Menu label="Actions for {title(v.id)}" items={menuOf(v)} align="end">
              {#snippet icon()}<Ellipsis size={14} />{/snippet}
            </Menu>
          </div>
          {@render itemForm(v)}
        </li>
      {/each}
    </ol>
    <p class="hint"><kbd class="mono">N</kbd> start next · <kbd class="mono">D</kbd> delay next +5 · <kbd class="mono">Ctrl+Z</kbd> undo</p>
  {/if}
</section>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: 0 var(--s-3) var(--s-2);
  }
  .panel:focus {
    outline: none;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .clock {
    font-size: var(--fs-lg);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .spacer {
    flex: 1;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    padding: var(--s-3);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  /* Live actions get large hit targets. */
  .big-targets {
    --control-h: 40px;
  }
  .card-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
  }
  .card-head .section-label {
    margin-right: auto;
  }
  .card-title {
    overflow: hidden;
    font-family: var(--font-heading);
    font-size: var(--fs-xl);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card-meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-1);
  }
  .starts {
    color: var(--accent-text);
    font-variant-numeric: tabular-nums;
  }
  .count {
    font-size: 40px;
    font-weight: 600;
    line-height: 1.1;
    font-variant-numeric: tabular-nums;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
    padding-top: var(--s-2);
  }
  kbd {
    padding: 0 4px;
    border-radius: 4px;
    box-shadow: inset 0 0 0 1px currentColor;
    font-size: var(--fs-xs);
    opacity: 0.75;
  }
  .nowline {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: var(--row-h);
    padding: 0 var(--s-1);
  }
  .now-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .items {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 20rem;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }
  .day {
    padding: var(--s-2) var(--s-1) 0;
  }
  .item {
    border-radius: var(--r-control);
  }
  .item:hover,
  .item:focus-within {
    background: var(--surface-2);
  }
  .item.is-now {
    background: var(--surface-3);
  }
  .item.is-now .title {
    font-weight: 600;
  }
  .item.is-past .time,
  .item.is-past .title {
    color: var(--text-muted);
  }
  .line {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: 40px;
    padding: var(--s-1);
  }
  .time {
    display: flex;
    flex: none;
    flex-direction: column;
    align-items: flex-end;
    width: 5ch;
    font-size: var(--fs-sm);
    font-variant-numeric: tabular-nums;
    line-height: 1.2;
  }
  .was {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .text {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    line-height: 1.25;
  }
  .title,
  .loc {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .struck {
    text-decoration: line-through;
  }
  .loc {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .badges {
    display: inline-flex;
    flex: none;
    gap: var(--s-1);
  }
  /* Quick actions: shown on hover or keyboard focus, over the badges. */
  .quick {
    position: absolute;
    right: calc(var(--control-h) + var(--s-5));
    display: inline-flex;
    gap: var(--s-1);
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    background: var(--surface-2);
    opacity: 0;
    pointer-events: none;
    transition: opacity var(--dur-fast) var(--ease);
  }
  .item.is-now .quick {
    background: var(--surface-3);
  }
  .item:hover .quick,
  .quick:focus-within {
    opacity: 1;
    pointer-events: auto;
  }
  .iform {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
    padding: var(--s-1) var(--s-2) var(--s-2) calc(5ch + var(--s-3));
  }
  .card .iform {
    padding: var(--s-2) 0 0;
  }
  .iform label {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .muted {
    color: var(--text-muted);
  }
  p.muted {
    margin: 0;
    font-size: var(--fs-sm);
  }
  .pad {
    padding: var(--s-2) 0;
  }
  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  @media (prefers-reduced-motion: reduce) {
    .quick {
      transition: none;
    }
  }
</style>

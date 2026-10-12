// The Polls page's state: the account's polls (from the server, only when the core may ask), the
// quota, this project's test polls (poll sources with test data), live updates (`poll-live`,
// `source-changed`) and the actions. Every failure becomes a toast in plain words (`pollError`);
// the core decides access, the page only displays it.

import { on, toAppError } from "$lib/api";
import * as api from "$lib/api/polls";
import { deleteSource, listSources } from "$lib/api/sources";
import { toasts } from "$lib/stores/toasts.svelte";
import type { PollConn, PollData, PollInput, PollQuota, PollSimulation, PollStatus, SourceSummary, TestPoll } from "$lib/types";
import { pollError } from "./model";

export type Selection = { kind: "poll"; id: string } | { kind: "test"; id: string } | { kind: "new" } | null;

export class PollsState {
  /** The account's polls, newest first (empty without access). */
  polls = $state.raw<PollData[]>([]);
  /** Null: not asked (no access) or not loaded yet. */
  quota = $state.raw<PollQuota | null>(null);
  /** This project's test polls (poll sources with test data), by name. */
  tests = $state.raw<SourceSummary[]>([]);
  /** Poll sources linked to a server poll: poll id → source id ("Use in a source" reuses it). */
  linked = $state.raw<ReadonlyMap<string, string>>(new Map());
  /** How each poll's results arrive (only polls with a stream or a watcher). */
  conn = $state.raw<Readonly<Record<string, PollConn>>>({});
  loaded = $state(false);
  /** "Show my existing polls" was asked this visit. */
  askedExisting = $state(false);
  selection = $state.raw<Selection>(null);
  busy = $state(false);

  #watched: string | null = null;

  /** Events and the first load; returns the teardown. */
  connect(): () => void {
    const offLive = on("poll-live", (l) => {
      this.conn = { ...this.conn, [l.id]: l.conn };
      if (l.poll) this.#put(l.poll);
      if (l.conn === "gone") this.#drop(l.id);
    });
    const offSource = on("source-changed", (s) => {
      if (s.kind === "poll") this.#putSource(s);
    });
    const offFiles = on("project-files-changed", (c) => {
      if (c.kind === "source") void this.loadSources();
    });
    void this.load();
    return () => {
      offLive();
      offSource();
      offFiles();
      this.watch(null);
    };
  }

  async load(): Promise<void> {
    await Promise.all([this.loadPolls(), this.loadSources()]);
    this.loaded = true;
    if (!this.selection) {
      const first = this.polls[0] ?? null;
      this.select(first ? { kind: "poll", id: first.id } : this.tests[0] ? { kind: "test", id: this.tests[0].id } : null);
    }
  }

  async loadPolls(onDemand = false): Promise<void> {
    try {
      const [polls, quota] = await Promise.all([api.listPolls(onDemand), api.pollsQuota()]);
      this.polls = polls;
      this.quota = quota;
      if (onDemand) this.askedExisting = true;
    } catch (e) {
      // Signed out or offline: the list stays as it was; the page explains with the lock.
      if (onDemand) toasts.push({ kind: "error", message: `Couldn't load your polls: ${pollError(e)}` });
    }
  }

  async loadSources(): Promise<void> {
    try {
      const all = (await listSources()).filter((s) => s.kind === "poll");
      this.tests = all.filter((s) => s.poll?.test).sort((a, b) => a.name.localeCompare(b.name));
      this.linked = new Map(all.flatMap((s) => (s.poll?.pollId ? [[s.poll.pollId, s.id] as const] : [])));
    } catch {
      // No project: no test polls.
    }
  }

  async refreshQuota(): Promise<void> {
    try {
      this.quota = await api.pollsQuota();
    } catch {
      // Cosmetic: the last numbers stay.
    }
  }

  get selectedPoll(): PollData | null {
    const s = this.selection;
    return s?.kind === "poll" ? (this.polls.find((p) => p.id === s.id) ?? null) : null;
  }

  get selectedTest(): SourceSummary | null {
    const s = this.selection;
    return s?.kind === "test" ? (this.tests.find((t) => t.id === s.id) ?? null) : null;
  }

  select(s: Selection): void {
    this.selection = s;
    this.watch(s?.kind === "poll" ? s.id : null);
  }

  /** Live results for the poll on screen (counted in the core: one watch at a time here). */
  watch(id: string | null): void {
    if (this.#watched === id) return;
    const prev = this.#watched;
    this.#watched = id;
    if (prev) void api.watchPoll(prev, false).catch(() => {});
    if (id) void api.watchPoll(id, true).catch(() => {});
  }

  #put(p: PollData): void {
    const i = this.polls.findIndex((x) => x.id === p.id);
    this.polls = i < 0 ? [p, ...this.polls] : this.polls.map((x) => (x.id === p.id ? p : x));
  }

  #drop(id: string): void {
    const i = this.polls.findIndex((x) => x.id === id);
    if (i < 0) return;
    this.polls = this.polls.filter((x) => x.id !== id);
    if (this.selection?.kind === "poll" && this.selection.id === id) {
      const next = this.polls[Math.min(i, this.polls.length - 1)];
      this.select(next ? { kind: "poll", id: next.id } : null);
    }
  }

  #putSource(s: SourceSummary): void {
    if (s.poll?.test) {
      const has = this.tests.some((t) => t.id === s.id);
      this.tests = (has ? this.tests.map((t) => (t.id === s.id ? s : t)) : [...this.tests, s]).sort((a, b) =>
        a.name.localeCompare(b.name),
      );
    }
  }

  /** Runs a poll command; a failure is a toast, the result goes into the list. */
  async #run(what: string, f: () => Promise<PollData | void>): Promise<PollData | null> {
    this.busy = true;
    try {
      const p = await f();
      if (p) this.#put(p);
      return p ?? null;
    } catch (e) {
      if (toAppError(e).code === "cloud" && toAppError(e).apiCode === "not_found") {
        const s = this.selection;
        if (s?.kind === "poll") this.#drop(s.id);
      }
      toasts.push({ kind: "error", message: `Couldn't ${what}: ${pollError(e)}` });
      return null;
    } finally {
      this.busy = false;
    }
  }

  async create(input: PollInput): Promise<PollData | null> {
    const p = await this.#run("create the poll", () => api.createPoll(input));
    if (p) {
      this.select({ kind: "poll", id: p.id });
      void this.refreshQuota();
    }
    return p;
  }

  update = (id: string, input: PollInput) => this.#run("save the poll", () => api.updatePoll(id, input));

  async open(id: string): Promise<void> {
    if (await this.#run("open the poll", () => api.openPoll(id))) void this.refreshQuota();
  }

  async close(id: string): Promise<void> {
    if (await this.#run("close the poll", () => api.closePoll(id))) void this.refreshQuota();
  }

  reset = (id: string) => this.#run("reset the votes", () => api.resetPoll(id));

  async remove(id: string): Promise<void> {
    this.busy = true;
    try {
      await api.deletePoll(id);
      this.#drop(id);
      void this.refreshQuota();
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't delete the poll: ${pollError(e)}` });
    } finally {
      this.busy = false;
    }
  }

  /** A new draft with the same question and answers (to run a closed or expiring poll again). */
  duplicate = (p: PollData) =>
    this.create({
      question: p.question,
      options: p.options.map((o) => o.label),
      showResults: p.showResults,
      ...(p.durationSeconds !== null ? { durationSeconds: p.durationSeconds } : {}),
    });

  /** The poll source showing `p` (made when there is none): bind it on any screen. */
  async useInSource(p: PollData): Promise<string | null> {
    const have = this.linked.get(p.id) ?? (await api.pollSourceFor(p.id).catch(() => null));
    if (have) return have;
    try {
      const doc = await api.createPollSource(p.question.slice(0, 100) || "Poll", { poll: p });
      this.linked = new Map([...this.linked, [p.id, doc.id]]);
      return doc.id;
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't make a source for this poll: ${toAppError(e).message}` });
      return null;
    }
  }

  // ---- Test polls ----

  async createTest(name: string, test: TestPoll): Promise<void> {
    try {
      const doc = await api.createPollSource(name, { test });
      await this.loadSources();
      this.select({ kind: "test", id: doc.id });
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't create the test poll: ${toAppError(e).message}` });
    }
  }

  async saveTest(id: string, question: string, options: string[]): Promise<void> {
    try {
      const doc = await api.getPollSource(id);
      if (!doc.test) return;
      await api.savePollSource({ ...doc, test: { ...doc.test, question, options } });
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't save the test poll: ${toAppError(e).message}` });
    }
  }

  simulate = (id: string, sim: PollSimulation) =>
    this.#test("simulate votes", () => api.simulateTestPoll(id, sim));
  resetTest = (id: string) => this.#test("reset the votes", () => api.resetTestPoll(id));
  setTestStatus = (id: string, status: PollStatus) =>
    this.#test(status === "open" ? "open the test poll" : "close the test poll", () => api.setTestPollStatus(id, status));

  async removeTest(id: string): Promise<void> {
    try {
      await deleteSource(id);
      const i = this.tests.findIndex((t) => t.id === id);
      this.tests = this.tests.filter((t) => t.id !== id);
      if (this.selection?.kind === "test" && this.selection.id === id) {
        const next = this.tests[Math.min(i, this.tests.length - 1)];
        this.select(next ? { kind: "test", id: next.id } : null);
      }
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't delete the test poll: ${toAppError(e).message}` });
    }
  }

  async #test(what: string, f: () => Promise<void>): Promise<void> {
    try {
      await f();
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't ${what}: ${toAppError(e).message}` });
    }
  }
}

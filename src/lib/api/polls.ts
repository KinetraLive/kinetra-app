// Live polls: the account's polls (quota, list, create, edit, open / close / reset / delete), live
// results while the Polls page shows one, poll sources and test polls. A domain module: only the
// Polls page and the poll source editor load it. The core decides access (no poll request at all
// for Free users) and checks every input; see core/polls.rs.

import type {
  PollData,
  PollInput,
  PollQuota,
  PollSimulation,
  PollStatus,
  PollSourceDoc,
  TestPoll,
} from "../types";
import { call } from "./index";

/** The plan's limits and this month's usage; null when no request may be made (Free). */
export const pollsQuota = () => call<PollQuota | null>("polls_quota");
/** The account's polls, newest first. `onDemand`: "Show my existing polls" (asks even without Pro). */
export const listPolls = (onDemand = false) => call<PollData[]>("list_polls", { onDemand });
export const createPoll = (input: PollInput) => call<PollData>("create_poll", { input });
export const updatePoll = (id: string, input: PollInput) => call<PollData>("update_poll", { id, input });
export const getPoll = (id: string) => call<PollData>("get_poll", { id });
export const openPoll = (id: string) => call<PollData>("open_poll", { id });
export const closePoll = (id: string) => call<PollData>("close_poll", { id });
export const resetPoll = (id: string) => call<PollData>("reset_poll", { id });
export const deletePoll = (id: string) => call<void>("delete_poll", { id });
/** Live results for this poll while shown (counted: pair every `true` with a `false`). */
export const watchPoll = (id: string, on: boolean) => call<void>("watch_poll", { id, on });

/** A poll source (check `SourceSummary.kind === "poll"` first). */
export const getPollSource = (id: string) => call<PollSourceDoc>("get_source", { id });
/** Saves a test poll's question and answers (votes and status are the simulation's). */
export const savePollSource = (doc: PollSourceDoc) => call<PollSourceDoc>("save_source", { doc });
/** A new poll source for a server poll or a test poll. */
export const createPollSource = (name: string, src: { poll: PollData } | { test: TestPoll }) =>
  call<PollSourceDoc>("create_poll_source", {
    name,
    poll: "poll" in src ? src.poll : null,
    test: "test" in src ? src.test : null,
  });
/** The source already showing this poll, if any. */
export const pollSourceFor = (pollId: string) => call<string | null>("poll_source_for", { pollId });
/** Whether the project shows a real poll (asks before taking test data to Program). */
export const projectUsesRealPolls = () => call<boolean>("project_uses_real_polls");

export const simulateTestPoll = (sourceId: string, sim: PollSimulation) =>
  call<void>("simulate_test_poll", { sourceId, sim });
export const resetTestPoll = (sourceId: string) => call<void>("reset_test_poll", { sourceId });
export const setTestPollStatus = (sourceId: string, status: PollStatus) =>
  call<void>("set_test_poll_status", { sourceId, status });

/** The QR code for a voting link as a `data:image/svg+xml` URL, made locally. */
export const pollQr = (text: string) => call<string>("poll_qr", { text });

/** "Add to screen": the Poll graphic on a screen, bound to the poll source. */
export const addPollToScreen = (sourceId: string, screenId: string) =>
  call<import("../types").ScreenState>("add_poll_to_screen", { sourceId, screenId });

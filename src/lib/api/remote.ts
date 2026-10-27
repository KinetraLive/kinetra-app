// Remote control (src-tauri/src/commands/remote.rs): the switch, Lock remote take, "Move it
// here?", a separate show, Test connection, Connection details and the show's collaborators.
// The core decides everything; events: `remote-control`, `remote-activity`, `remote-diag`,
// `cue-position`.

import type { Collaborator, Collaborators, CuePosition, RemoteActivity, RemoteDiag, RemoteTestResult, RemoteView } from "../types";
import { call } from "./index";

export const remoteView = () => call<RemoteView>("remote_view");
/** On or off for the open project (this session only). On needs Pro and a signed-in account. */
export const remoteSet = (on: boolean) => call<RemoteView>("remote_set", { on });
/** After the run ended or remote control stopped. */
export const remoteStartAgain = () => call<RemoteView>("remote_start_again");
/** Ends the run for everyone; the switch stays on (Start again). */
export const remoteEnd = () => call<RemoteView>("remote_end");
export const remoteLock = (on: boolean) => call<RemoteView>("remote_lock", { on });
/** "Move it here?" → Move it here. */
export const remoteTakeover = () => call<void>("remote_takeover");
/** A new project ID: a show of its own (empty crew); the other copy keeps the old one. */
export const remoteNewShow = () => call<RemoteView>("remote_new_show");
/** Recent remote actions of this run, newest first. */
export const remoteActivity = () => call<RemoteActivity[]>("remote_activity");
export const cuePosition = () => call<CuePosition>("cue_position");
export const remoteTest = () => call<RemoteTestResult>("remote_test");
/** Counted: while ≥ 1 watch, `remote-diag` follows changes (≤ 2/s). Returns the view now. */
export const remoteDiagWatch = (on: boolean) => call<RemoteDiag>("remote_diag_watch", { on });
export const remoteExportDiagnostics = (path: string) => call<void>("remote_export_diagnostics", { path });

// Collaborators (protocol 2 §9; the owner's own show). Errors carry the API's code
// (`collaborator_limit` with `details.max`, `already_collaborator`, …).
export const remoteCollaborators = () => call<Collaborators>("remote_collaborators");
/** The server sends the invite email. Unknown codes are dropped by the core. */
export const remoteInvite = (email: string, permissions: string[]) =>
  call<Collaborator>("remote_invite", { email, permissions });
export const remoteCollaboratorSet = (id: string, permissions: string[]) =>
  call<Collaborator>("remote_collaborator_set", { id, permissions });
/** Revokes a pending invite, or removes a joined collaborator (disconnected at once). */
export const remoteCollaboratorRemove = (id: string) => call<void>("remote_collaborator_remove", { id });

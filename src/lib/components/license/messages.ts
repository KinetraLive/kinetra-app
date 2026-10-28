// Human messages for every error code in docs/API.md (licensing, sign-in and the general table),
// plus the live license-key formatting from kinetra-license's `key_format::format_partial`.
// The core checks keys for real (`key_format::normalize`); this only formats the field.

import { toAppError } from "$lib/api";
import type { RedeemResult } from "$lib/types";

/** Every API.md code → what the operator reads. `null` = show the server's own `message`
 * (API.md says "Show `message`"). */
export const API_MESSAGES: Readonly<Record<string, string | null>> = {
  bad_request: null,
  authorization_pending: "Waiting for you to approve the sign-in in your browser.",
  slow_down: "Waiting for you to approve the sign-in in your browser.",
  expired_token: "The sign-in code expired. Start again.",
  access_denied: "Sign-in was denied.",
  unauthorized: "Sign in to continue.",
  invalid_token: "You were signed out. Sign in again.",
  invalid_credentials: "Sign-in failed. Request a new link.",
  forbidden: "Your account can't do this.",
  not_found: "Not found. It may have been removed already.",
  method_not_allowed: "Kinetra sent a request the server doesn't accept. Update Kinetra.",
  timeout: "The server took too long. Try again later.",
  conflict: null,
  payload_too_large: "That's too large to send.",
  unsupported_media_type: "Kinetra sent a request the server doesn't accept. Update Kinetra.",
  rate_limited: "Too many requests. Try again shortly.",
  internal_error: "Something went wrong on our side. Try again later.",
  service_unavailable: "The Kinetra service is temporarily unavailable. Your work isn't affected.",
  clock_skew: "Your computer's date or time is wrong. Fix it, then try again.",
  replayed_request: "That request was already used. Try again.",
  invalid_license_key: null,
  no_license: "No usable license on this account. Sign in again.",
  license_expired: "Your Pro subscription ended. Renew it to use Pro features.",
  license_revoked: "This license was revoked. Contact hello@kinetra.live.",
  seats_full: "All computers on this license are in use.",
  backup_in_use: "This license already has a backup computer.",
  backup_not_included: "This plan doesn't include a backup computer.",
  activation_revoked: "This computer was deactivated. Activate it again.",
  active_license: null,
  license_service_unavailable: "Licensing is temporarily unavailable. Your license keeps working.",
  // AI (part 3).
  ai_input_too_long: null,
  ai_disabled_for_account: "AI is turned off for this account. Contact hello@kinetra.live.",
  ai_quota_exhausted: null,
  ai_busy: "A template is already being generated. Wait for it to finish.",
  ai_paused: "AI is paused for the rest of the month.",
  ai_disabled: "AI is turned off right now.",
  ai_declined: "Couldn't generate this template. Try rephrasing.",
  ai_output_invalid: "The result wasn't a valid template. Try again.",
  ai_incomplete: "The result was cut off. Try again.",
  ai_unavailable: "AI is unavailable right now. Try again later.",
  ai_job_gone: "That result is no longer available. Generate it again.",
  // Polls. The Polls page adds the numbers and dates from `details` (max, resets_at).
  entitlement_required: "Creating and opening polls is part of Kinetra Pro. Your existing polls still work.",
  poll_open: "Close the poll to change it.",
  poll_has_votes: "Reset the votes to change the question or answers.",
  poll_not_open: "This poll isn't open.",
  poll_limit_reached: "You have as many polls open as your plan allows. Close one to open this.",
  poll_monthly_limit_reached: null,
  poll_expired: "This poll is about to be deleted. Duplicate it to run it again.",
  // Cloud backup. Quiet: the project on this computer is never affected.
  invalid_path: "A file name can't be backed up. It's listed in the backup report.",
  too_many_files: "This project has more than 50,000 files. Cloud backup can't hold it.",
  manifest_too_large: "This project has too many files for one cloud backup.",
  file_too_large: "A file is too large for cloud backup. It's listed in the backup report.",
  cloud_not_in_plan: "Cloud backup is part of Kinetra Pro.",
  cloud_read_only: "Your plan no longer includes cloud backup. Your backups can still be downloaded.",
  storage_full: "Cloud storage is full. Your project is saved on this computer; cloud backup is paused.",
  daily_upload_limit: "Today's cloud upload allowance is used. Backup continues tomorrow.",
  project_in_trash: "This project's cloud backup is in the trash.",
  too_many_projects: "You have as many projects in the cloud as your plan allows.",
  too_many_kept: "You're keeping as many versions as your plan allows.",
  missing_blobs: "The backup wasn't complete. Kinetra will try again.",
  upload_mismatch: "A file changed during the backup. Kinetra will try again.",
  cannot_delete_head: "The newest version can't be deleted. Delete the project's backup instead.",
  cloud_unavailable: "Cloud backup: offline, will retry.",
  commit_incomplete: "Cloud backup is still checking the uploaded files.",
  // Remote control collaborators (the invite dialog words collaborator_limit with details.max).
  unknown_permission: "Kinetra sent a permission the server doesn't know. Update Kinetra.",
  already_collaborator: "Already a collaborator. Change their permissions instead.",
  collaborator_limit: "Your plan's collaborators per show are all used.",
};

/** The message for a failed licensing / sign-in call. */
export function cloudMessage(e: unknown): string {
  const err = toAppError(e);
  if (err.code === "cloud" && err.apiCode) {
    const m = API_MESSAGES[err.apiCode];
    if (m === null) return err.message;
    if (m !== undefined) return m;
  }
  return err.message;
}

/** `license.json` couldn't be written (the license applies until Kinetra restarts). */
export const saveFailedText = (n: { reason: string; folder: string }) =>
  `Couldn't save the license on this computer: ${n.reason.replace(/\.$/, "")}. Check that Kinetra can write to ${n.folder}.`;

/** What a redeemed key did here: the toast after Add key. */
export function redeemText(r: RedeemResult): { kind: "info" | "error"; message: string } {
  switch (r.outcome) {
    case "moved":
      return { kind: "info", message: "License key redeemed." };
    case "saveFailed":
      return { kind: "error", message: "License key redeemed, but Kinetra couldn't save it on this computer. See Preferences › License." };
    case "blocked":
      return { kind: "info", message: "License key redeemed. Free a computer to use it here." };
    case "kept":
      return {
        kind: "info",
        message: r.planName ? `Redeemed. This computer already uses ${r.planName}.` : "Redeemed. This computer keeps its current license.",
      };
    case "offline":
      return { kind: "info", message: "License key redeemed. This computer switches to it once Kinetra is back online." };
  }
}

// ---- License keys (KNTR-XXXX-XXXX-XXXX) -------------------------------------------------------

const PREFIX = "KNTR";
const ALPHABET = "23456789ABCDEFGHJKMNPQRSTVWXYZ";
const BODY_LEN = 12;

/** Live formatting for the key field (`kntr4b7` → `KNTR-4B7`): invalid characters dropped,
 * cut at 12 characters. Mirrors `key_format::format_partial`. */
export function formatKey(input: string): string {
  let upper = input.replace(/[\s\-–—]/g, "").toUpperCase();
  if (upper.startsWith(PREFIX) && upper.length !== BODY_LEN) upper = upper.slice(PREFIX.length);
  const body = [...upper].filter((c) => ALPHABET.includes(c)).slice(0, BODY_LEN).join("");
  if (!body) return "";
  return `${PREFIX}-${body.match(/.{1,4}/g)!.join("-")}`;
}

/** The key is complete (12 characters); the core checks it. */
export const keyComplete = (formatted: string) => formatted.replace(/^KNTR-|-/g, "").length === BODY_LEN;

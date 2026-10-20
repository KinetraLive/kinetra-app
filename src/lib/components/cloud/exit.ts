// "Back up before quitting?" (the app plan's decision 6): asked only when the open project has
// changes not in the cloud and nothing is on air (the core decides: `cloud_exit_should_ask`);
// otherwise quitting never waits. "Back up and quit" starts a backup and quits when it ends,
// with a sticky toast offering "Quit now". Loaded lazily by the exit command.

import { on } from "$lib/api";
import { cloudBackupBeforeExit, cloudExitShouldAsk } from "$lib/api/cloud-backup";
import { t } from "$lib/i18n";
import { toasts } from "$lib/stores/toasts.svelte";

/**
 * Asks, and backs up when the user wants that. `true`: this took over (canceled, or it quits by
 * itself once the backup ends); `false`: quit now.
 */
export async function backUpBeforeQuit(quit: () => Promise<void>): Promise<boolean> {
  let ask = false;
  try {
    ask = await cloudExitShouldAsk();
  } catch {
    // Can't tell: never hold up quitting.
  }
  if (!ask) return false;
  const { message } = await import("@tauri-apps/plugin-dialog");
  const BACK = t("Back up and quit");
  const QUIT = t("Quit");
  const CANCEL = t("Cancel");
  const choice = await message(t("This project has changes that aren't in the cloud yet."), {
    title: t("Back up before quitting?"),
    kind: "info",
    buttons: { yes: BACK, no: QUIT, cancel: CANCEL },
  });
  if (choice === CANCEL || choice === "Cancel") return true;
  if (choice !== BACK && choice !== "Yes") return false;
  try {
    const v = await cloudBackupBeforeExit();
    if (!v.project?.run) return false;
  } catch {
    return false;
  }
  let done = false;
  const finish = () => {
    if (done) return;
    done = true;
    off();
    toasts.dismiss(toast);
    void quit();
  };
  const toast = toasts.push({
    kind: "info",
    message: t("Backing up before quitting…"),
    timeoutMs: 0,
    action: { label: t("Quit now"), run: finish },
  });
  const off = on("cloud-changed", (v) => {
    if (!v.project?.run) finish();
  });
  return true;
}

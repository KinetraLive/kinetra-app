// Media imports with progress: the system file dialog → `import_media` (a job copying the files
// into assets/ on the backend) → `asset-import` events per file (≤ 10/s) until every file is done
// (copied, failed or cancelled). One shared instance, so the library and the pickers show the
// same jobs; the event listener exists only while a job runs.

import { on, toAppError } from "$lib/api";
import { cancelImport, importMedia } from "$lib/api/media";
import { toasts } from "$lib/stores/toasts.svelte";
import type { AssetImportProgress, MediaKind } from "$lib/types";
import { MEDIA_EXTENSIONS } from "./media";

export interface ImportJob {
  id: string;
  /** File names in the order they were picked (the base names of the paths; replaced by the
   * name in assets/ once the core reports it). */
  names: string[];
  /** Latest progress per file, by its position in `names` (`AssetImportProgress.index`). */
  files: Record<number, AssetImportProgress>;
  cancelling: boolean;
}

/** The base name of a path from the file dialog (either slash). */
export const baseName = (p: string) => p.slice(Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\")) + 1);

/** Total copied / total bytes of a job (files without an event yet count as 0 / 0). */
export function jobBytes(job: ImportJob): { copied: number; total: number } {
  let copied = 0;
  let total = 0;
  for (const f of Object.values(job.files)) {
    copied += f.copied;
    total += f.total;
  }
  return { copied, total };
}

/** Every file reported done. */
export const jobDone = (job: ImportJob) => job.names.every((_, i) => job.files[i]?.done);

class Importer {
  jobs = $state.raw<ImportJob[]>([]);
  /** Bumped when a job finishes (lists refresh on it). */
  finished = $state(0);
  /** Names imported by the last finished job, for selecting them. */
  lastImported = $state.raw<string[]>([]);
  #off: (() => void) | null = null;
  #picking = false;
  /** Events of a job whose id hasn't come back yet (the reply can trail the first events). */
  #early = new Map<string, AssetImportProgress[]>();

  get busy() {
    return this.jobs.length > 0;
  }

  #listen() {
    this.#off ??= on("asset-import", (p) => this.#progress(p));
  }

  #progress(p: AssetImportProgress) {
    const job = this.jobs.find((j) => j.id === p.job);
    if (!job) {
      this.#early.set(p.job, [...(this.#early.get(p.job) ?? []), p]);
      return;
    }
    const names = [...job.names];
    names[p.index] = p.name;
    const next: ImportJob = { ...job, names, files: { ...job.files, [p.index]: p } };
    this.jobs = this.jobs.map((j) => (j.id === p.job ? next : j));
    if (jobDone(next)) this.#finish(next);
  }

  #finish(job: ImportJob) {
    this.jobs = this.jobs.filter((j) => j.id !== job.id);
    if (this.jobs.length === 0) {
      this.#off?.();
      this.#off = null;
    }
    const files = job.names.map((_, i) => job.files[i]);
    const ok = files.filter((f) => f && !f.error).map((f) => f.name);
    const failed = files.filter((f) => f?.error && !/cancel/i.test(f.error));
    this.lastImported = ok;
    this.finished++;
    if (failed.length > 0) {
      toasts.push({
        kind: "error",
        message: failed.length === 1 ? `Couldn't import “${failed[0].name}”: ${failed[0].error}` : `Couldn't import ${failed.length} files.`,
      });
    }
    if (ok.length > 0) {
      toasts.push({ kind: "info", message: ok.length === 1 ? `Imported “${ok[0]}”.` : `Imported ${ok.length} files.` });
    }
  }

  /** Opens the file dialog for these kinds and starts the import. */
  async pickAndImport(kinds: readonly MediaKind[]): Promise<void> {
    if (this.#picking) return;
    this.#picking = true;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const exts = kinds.flatMap((k) => MEDIA_EXTENSIONS[k]);
      const label = kinds.length === 1 ? { video: "Video", audio: "Audio", image: "Images" }[kinds[0]] : "Media";
      const picked = await open({ multiple: true, filters: [{ name: label, extensions: [...exts] }] });
      const paths = picked === null ? [] : Array.isArray(picked) ? picked : [picked];
      if (paths.length > 0) await this.start(paths);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't import: ${toAppError(e).message}` });
    } finally {
      this.#picking = false;
    }
  }

  /** Starts importing these files (absolute paths). */
  async start(paths: readonly string[]): Promise<void> {
    this.#listen();
    const names = paths.map(baseName);
    let id: string;
    try {
      id = await importMedia([...paths]);
    } catch (e) {
      if (this.jobs.length === 0) {
        this.#off?.();
        this.#off = null;
      }
      throw e;
    }
    this.jobs = [...this.jobs, { id, names, files: {}, cancelling: false }];
    const early = this.#early.get(id) ?? [];
    this.#early.clear();
    for (const p of early) this.#progress(p);
  }

  async cancel(id: string): Promise<void> {
    this.jobs = this.jobs.map((j) => (j.id === id ? { ...j, cancelling: true } : j));
    try {
      await cancelImport(id);
    } catch (e) {
      this.jobs = this.jobs.map((j) => (j.id === id ? { ...j, cancelling: false } : j));
      toasts.push({ kind: "error", message: `Couldn't cancel the import: ${toAppError(e).message}` });
    }
  }
}

export const importer = new Importer();

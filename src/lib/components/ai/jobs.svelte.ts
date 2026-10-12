// AI jobs of this session, mirrored from `ai-job` events for the job chip (main window).

import { on } from "$lib/api";
import { aiJobs } from "$lib/api/ai";
import type { AiJobView } from "$lib/types";

class Jobs {
  list = $state.raw<AiJobView[]>([]);
  /** Jobs the operator closed in the chip. */
  hidden = $state.raw<ReadonlySet<string>>(new Set());
  #off: (() => void) | null = null;

  start() {
    if (this.#off) return;
    this.#off = on("ai-job", (j) => this.put(j));
    aiJobs().then(
      (l) => (this.list = l),
      () => {},
    );
  }

  put(j: AiJobView) {
    const rest = this.list.filter((x) => x.id !== j.id);
    this.list = [j, ...rest];
  }

  hide(id: string) {
    this.hidden = new Set([...this.hidden, id]);
  }

  get visible(): AiJobView[] {
    return this.list.filter((j) => !this.hidden.has(j.id));
  }
}

export const jobs = new Jobs();

// AI templates. The core gates every start (Pro, full access) and checks the input; limits come
// from the core (`aiLimits`).

import type { AiGenerateInput, AiJobView, AiLimits, AiQuota } from "../types";
import { call } from "./index";

export const aiLimits = () => call<AiLimits>("ai_limits");
export const aiQuota = () => call<AiQuota>("ai_quota");
export const aiGenerate = (input: AiGenerateInput) => call<AiJobView>("ai_generate", { input });
export const aiTweak = (templateId: string, instruction: string) =>
  call<AiJobView>("ai_tweak", { templateId, instruction });
export const aiCancel = (jobId: string) => call<void>("ai_cancel", { jobId });
export const aiJobs = () => call<AiJobView[]>("ai_jobs");

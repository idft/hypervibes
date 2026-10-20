import type { SubagentRun } from "../db/subagent-schema.js";

export const runHistoryPageSize = 50;
export type RunSummary = Pick<SubagentRun, "id" | "subagentId" | "source" | "task" | "status" | "createdAt">;
export interface RunPagination { page: number; pages: number; total: number; pageSize: number }
export interface InboxRequestSummary { id: string; intent: string; reason: string; preview: string; createdAt: number; discardedReason: string | null }
export interface RunHistory { inbox?: InboxRequestSummary[]; runs: RunSummary[]; pagination: RunPagination; schedulerError: string | null; schedule?: { nextRunAt: number | null; updatedAt: number } }
export function runPagination(total: number, requestedPage = 1): RunPagination {
  const pages = Math.max(1, Math.ceil(total / runHistoryPageSize));
  return { page: Math.min(requestedPage, pages), pages, total, pageSize: runHistoryPageSize };
}

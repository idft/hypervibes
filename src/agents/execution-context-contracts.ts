import type { LearningSnapshot } from "../memory/learning-contracts.js";
export type ExecutionOrigin =
  | { mode: "user_directed"; userId: string; userInputRequestIds: string[]; expiresAt: number }
  | { mode: "automatic"; reason: "schedule" | "worker_report" | "unverified" | "mixed_inputs" };
export interface RunContext {
  version: 1; origin: ExecutionOrigin; instructions: string; scheduledBoundary: number | null;
  learnings: { agent: LearningSnapshot; worker: LearningSnapshot } | null;
  learningStatus: "snapshot" | "legacy/none" | "not_admitted";
}
export const automaticOrigin: ExecutionOrigin = { mode: "automatic", reason: "unverified" };

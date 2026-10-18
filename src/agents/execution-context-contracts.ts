import type { LearningSnapshot } from "../memory/learning-contracts.js";

/** Immutable per-run snapshot: exact injected instructions plus frozen learnings. */
export interface RunContext {
  version: 2; instructions: string;
  learnings: { agent: LearningSnapshot; worker: LearningSnapshot } | null;
  learningStatus: "snapshot" | "legacy/none" | "not_admitted";
}
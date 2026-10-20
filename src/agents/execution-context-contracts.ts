import type { InsightSnapshot } from "../memory/insight-contracts.js";

/** Immutable per-run snapshot: exact injected instructions plus frozen insights. */
export interface RunContext {
  version: 2; instructions: string;
  insights: { agent: InsightSnapshot; worker: InsightSnapshot } | null;
  insightStatus: "snapshot" | "legacy/none" | "not_admitted";
}
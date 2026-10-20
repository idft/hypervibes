import type { SubagentRow, SubagentRun } from "../db/subagent-schema.js";

export interface DashboardSubagent extends Pick<SubagentRow, "id" | "name" | "enabled"> {
  runs: Pick<SubagentRun, "id" | "status" | "createdAt" | "result">[];
}

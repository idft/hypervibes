import { sql } from "drizzle-orm";
import type { RunContext } from "../agents/execution-context-contracts.js";
import { check, foreignKey, sqliteTable, text } from "drizzle-orm/sqlite-core";
import { hvSubagentRuns } from "./subagent-schema.js";

export const hvSubagentRunContexts = sqliteTable("hv_subagent_run_contexts", {
  agentId: text("agent_id").notNull(), runId: text("run_id").primaryKey(), context: text("context", { mode: "json" }).$type<RunContext>().notNull(),
}, (t) => [foreignKey({ columns: [t.agentId, t.runId], foreignColumns: [hvSubagentRuns.agentId, hvSubagentRuns.id] }).onDelete("cascade"), check("hv_run_context_bound", sql`length(${t.context}) <= 120000`)]);

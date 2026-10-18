import { sql } from "drizzle-orm";
import type { ExecutionOrigin, RunContext } from "../agents/execution-context-contracts.js";
import { check, foreignKey, integer, primaryKey, sqliteTable, text, uniqueIndex } from "drizzle-orm/sqlite-core";
import { hvAgents } from "./agent-schema.js";
import { hvSubagentRuns } from "./subagent-schema.js";
import { hvMemories } from "./memory-schema.js";
import { hvOrderRequests } from "./order-schema.js";

export const hvAgentInputOrigins = sqliteTable("hv_agent_input_origins", {
  agentId: text("agent_id").notNull().references(() => hvAgents.id), requestId: text("request_id").notNull(), bodyHash: text("body_hash").notNull(), userId: text("user_id").notNull(), admittedAt: integer("admitted_at").notNull(), expiresAt: integer("expires_at").notNull(),
}, (t) => [primaryKey({ columns: [t.agentId, t.requestId] }), check("hv_input_deadline", sql`${t.expiresAt} > ${t.admittedAt}`)]);
export const hvAgentExecutionFrames = sqliteTable("hv_agent_execution_frames", {
  agentId: text("agent_id").notNull().references(() => hvAgents.id), generationTaskId: integer("generation_task_id").notNull(), conversationId: integer("conversation_id").notNull(),
  inputScopeKey: text("input_scope_key").notNull(), inputs: text("inputs").notNull(), runId: text("run_id"), origin: text("origin", { mode: "json" }).$type<ExecutionOrigin>().notNull(), config: text("config").notNull(), createdAt: integer("created_at").notNull(), expiresAt: integer("expires_at").notNull(),
}, (t) => [primaryKey({ columns: [t.agentId, t.generationTaskId] }), foreignKey({ columns: [t.agentId, t.runId], foreignColumns: [hvSubagentRuns.agentId, hvSubagentRuns.id] })]);
export const hvSubagentRunContexts = sqliteTable("hv_subagent_run_contexts", {
  agentId: text("agent_id").notNull(), runId: text("run_id").primaryKey(), context: text("context", { mode: "json" }).$type<RunContext>().notNull(),
}, (t) => [foreignKey({ columns: [t.agentId, t.runId], foreignColumns: [hvSubagentRuns.agentId, hvSubagentRuns.id] }), check("hv_run_context_bound", sql`length(${t.context}) <= 120000`)]);
export const hvOrderExecutionContexts = sqliteTable("hv_order_execution_contexts", {
  agentId: text("agent_id").notNull(), requestId: text("request_id").primaryKey(), decisionNumber: integer("decision_number"), generationTaskId: integer("generation_task_id").notNull(), runId: text("run_id"), origin: text("origin").notNull(), policy: text("policy").notNull(), slotKey: text("slot_key"),
}, (t) => [foreignKey({ columns: [t.agentId, t.requestId], foreignColumns: [hvOrderRequests.agentId, hvOrderRequests.id] }), foreignKey({ columns: [t.agentId, t.decisionNumber], foreignColumns: [hvMemories.agentId, hvMemories.number] }), foreignKey({ columns: [t.agentId, t.generationTaskId], foreignColumns: [hvAgentExecutionFrames.agentId, hvAgentExecutionFrames.generationTaskId] })]);
export const hvAutomaticEntryClaims = sqliteTable("hv_automatic_entry_claims", {
  agentId: text("agent_id").notNull(), slotKey: text("slot_key").notNull(), requestId: text("request_id").notNull(),
}, (t) => [primaryKey({ columns: [t.agentId, t.slotKey] }), foreignKey({ columns: [t.agentId, t.requestId], foreignColumns: [hvOrderRequests.agentId, hvOrderRequests.id] }), uniqueIndex("hv_entry_claim_request_idx").on(t.agentId, t.requestId)]);

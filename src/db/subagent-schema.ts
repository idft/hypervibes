import { sql } from "drizzle-orm";
import { check, foreignKey, index, integer, primaryKey, sqliteTable, text, uniqueIndex } from "drizzle-orm/sqlite-core";
import { hvAgents } from "./agent-schema.js";
import type { WorkerTriggers, WorkerRequestPayload } from "../agents/worker-triggers.js";

export const hvSubagents = sqliteTable("hv_subagents", {
  id: text("id").primaryKey(),
  agentId: text("agent_id").notNull().references(() => hvAgents.id, { onDelete: "restrict" }),
  kind: text("kind", { enum: ["orchestrator", "worker"] }).notNull(),
  name: text("name").notNull(),
  description: text("description").notNull().default(""),
  instructions: text("instructions").notNull(),
  provider: text("provider").notNull(),
  modelId: text("model_id").notNull(),
  thinkingLevel: text("thinking_level").notNull().default(""),
  tools: text("tools", { mode: "json" }).$type<string[]>().notNull(),
  triggers: text("triggers", { mode: "json" }).$type<WorkerTriggers>().notNull().default({ inboxEnabled: false, workerDelegates: [], researchSources: [], fills: false, orderChanges: false }),
  templateOrigin: text("template_origin", { mode: "json" }).$type<{ id: string; version: number; hash: string }>(),
  enabled: integer("enabled", { mode: "boolean" }).notNull().default(true),
  archived: integer("archived", { mode: "boolean" }).notNull().default(false),
  delegatable: integer("delegatable", { mode: "boolean" }).notNull().default(true),
  persistent: integer("persistent", { mode: "boolean" }).notNull().default(false),
  conversationId: integer("conversation_id"),
  schedule: text("schedule").notNull().default(""),
  delaySeconds: integer("delay_seconds").notNull().default(1),
  nextRunAt: integer("next_run_at"),
  completion: text("completion", { enum: ["save", "notify", "wake"] }).notNull().default("wake"),
  timeoutSeconds: integer("timeout_seconds").notNull().default(900),
  createdAt: integer("created_at").notNull(),
  updatedAt: integer("updated_at").notNull(),
}, (t) => [
  index("hv_subagents_agent_idx").on(t.agentId),
  uniqueIndex("hv_subagents_identity_idx").on(t.agentId, t.id),
  uniqueIndex("hv_subagents_main_idx").on(t.agentId).where(sql`${t.kind} = 'orchestrator'`),
  check("hv_subagents_main_protected", sql`${t.kind} != 'orchestrator' OR (${t.enabled} = 1 AND ${t.archived} = 0 AND ${t.schedule} = '')`),
]);

export const hvSubagentRuns = sqliteTable("hv_subagent_runs", {
  id: text("id").primaryKey(),
  agentId: text("agent_id").notNull().references(() => hvAgents.id, { onDelete: "restrict" }),
  subagentId: text("subagent_id").notNull().references(() => hvSubagents.id, { onDelete: "restrict" }),
  requestId: text("request_id").notNull(),
  source: text("source", { enum: ["manual", "schedule", "delegation", "event"] }).notNull(),
  task: text("task").notNull(),
  config: text("config", { mode: "json" }).$type<SubagentRow>().notNull(),
  status: text("status", { enum: ["queued", "running", "completed", "failed", "cancelled", "skipped"] }).notNull().default("queued"),
  taskId: integer("task_id"),
  conversationId: integer("conversation_id"),
  result: text("result").notNull().default(""),
  createdAt: integer("created_at").notNull(),
  finishedAt: integer("finished_at"),
  reportedAt: integer("reported_at"),
}, (t) => [
  uniqueIndex("hv_subagent_runs_request_idx").on(t.agentId, t.requestId),
  uniqueIndex("hv_subagent_runs_producer_idx").on(t.agentId, t.id, t.subagentId),
  uniqueIndex("hv_subagent_runs_identity_idx").on(t.agentId, t.id),
  uniqueIndex("hv_subagent_runs_active_idx").on(t.subagentId).where(sql`${t.status} IN ('queued', 'running')`),
  index("hv_subagent_runs_history_idx").on(t.subagentId, t.createdAt),
]);
/** Activation windows are maintained atomically by the inbox migration's SQLite triggers. */
export const hvWorkerSubscriptions = sqliteTable("hv_worker_subscriptions", {
  subagentId: text("subagent_id").notNull().references(() => hvSubagents.id),
  eventType: text("event_type").notNull(),
  sourceId: text("source_id").notNull(),
  activatedAt: integer("activated_at").notNull(),
}, (t) => [primaryKey({ columns: [t.subagentId, t.eventType, t.sourceId] })]);

/** Application-owned inbox and request provenance; admission atomically links requests to runs. */
export const hvWorkerRequests = sqliteTable("hv_worker_requests", {
  id: text("id").primaryKey(),
  agentId: text("agent_id").notNull().references(() => hvAgents.id, { onDelete: "restrict" }),
  subagentId: text("subagent_id").notNull(),
  eventKey: text("event_key").notNull(),
  payload: text("payload", { mode: "json" }).$type<WorkerRequestPayload>().notNull(),
  createdAt: integer("created_at").notNull(),
  runId: text("run_id"),
  discardedReason: text("discarded_reason"),
}, (t) => [
  foreignKey({ columns: [t.agentId, t.subagentId], foreignColumns: [hvSubagents.agentId, hvSubagents.id] }),
  foreignKey({ columns: [t.agentId, t.runId, t.subagentId], foreignColumns: [hvSubagentRuns.agentId, hvSubagentRuns.id, hvSubagentRuns.subagentId] }),
  uniqueIndex("hv_worker_requests_event_idx").on(t.agentId, t.subagentId, t.eventKey),
  index("hv_worker_requests_pending_idx").on(t.subagentId, t.createdAt).where(sql`${t.runId} IS NULL AND ${t.discardedReason} IS NULL`),
]);
export type WorkerRequest = typeof hvWorkerRequests.$inferSelect;
export type SubagentRow = typeof hvSubagents.$inferSelect;
export type SubagentRun = typeof hvSubagentRuns.$inferSelect;

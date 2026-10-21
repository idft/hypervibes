import { foreignKey, index, integer, sqliteTable, text } from "drizzle-orm/sqlite-core";
import { hvOrderRequests } from "./order-schema.js";
import { hvSubagents, hvSubagentRuns } from "./subagent-schema.js";

/** Host-captured origin, independent of the optional model-authored decision ref. */
export const hvOrderRequestProvenance = sqliteTable("hv_order_request_provenance", {
  requestId: text("request_id").primaryKey(),
  agentId: text("agent_id").notNull(),
  producerSubagentId: text("producer_subagent_id").notNull(),
  sourceRunId: text("source_run_id"),
  conversationId: integer("conversation_id").notNull(),
  toolTaskId: integer("tool_task_id").notNull(),
}, (t) => [
  foreignKey({ columns: [t.agentId, t.requestId], foreignColumns: [hvOrderRequests.agentId, hvOrderRequests.id] }).onDelete("cascade"),
  foreignKey({ columns: [t.agentId, t.producerSubagentId], foreignColumns: [hvSubagents.agentId, hvSubagents.id] }),
  foreignKey({ columns: [t.agentId, t.sourceRunId, t.producerSubagentId], foreignColumns: [hvSubagentRuns.agentId, hvSubagentRuns.id, hvSubagentRuns.subagentId] }),
  index("hv_order_provenance_run_idx").on(t.agentId, t.sourceRunId),
]);

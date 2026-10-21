import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { agentOperation, agentUser, field } from "./agents.js";
import { requestId } from "../agents/agent-input.js";
import { fields, integer } from "../memory/validation.js";
import { insightScope } from "../memory/insight-contracts.js";

export const listMemories = createServerFn({ method: "GET" }).validator((data: unknown) => { const row = fields(data, ["agentId", "filters"]); return { agentId: field(data, "agentId"), filters: row.filters ?? {} }; }).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentOperation(async () => { await context.runtime.agents.owned(user.id, data.agentId); return context.runtime.agents.execution.memories.list(data.agentId, data.filters); });
});
export const getMemory = createServerFn({ method: "GET" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), ref: field(data, "ref") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentOperation(async () => { await context.runtime.agents.owned(user.id, data.agentId); return context.runtime.agents.execution.memories.get(data.agentId, data.ref); });
});
export const getInsights = createServerFn({ method: "GET" }).validator((data: unknown) => { const row = fields(data, ["agentId", "scope", "version", "history", "beforeVersion"]); if (row.history !== undefined && typeof row.history !== "boolean") throw new Error("Invalid history flag"); return { agentId: field(data, "agentId"), scope: insightScope(row.scope), version: row.version == null ? null : integer(row.version, 1, Number.MAX_SAFE_INTEGER - 1, "revision version"), history: row.history === true, beforeVersion: row.beforeVersion == null ? null : integer(row.beforeVersion, 1, Number.MAX_SAFE_INTEGER, "history cursor") }; }).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentOperation(async () => {
    await context.runtime.agents.owned(user.id, data.agentId);
    const service = context.runtime.agents.execution.insights;
    const current = await service.current(data.agentId, data.scope);
    return { current, head: current.version === 0 ? null : await service.revision(data.agentId, data.scope, current.version), revision: data.version == null ? null : await service.revision(data.agentId, data.scope, data.version), history: data.history ? await service.history(data.agentId, data.scope, data.beforeVersion === null ? {} : { beforeVersion: data.beforeVersion }) : null };
  });
});
export const replaceInsights = createServerFn({ method: "POST" }).validator((data: unknown) => { const row = fields(data, ["agentId", "requestId", "input"]); return { agentId: field(data, "agentId"), requestId: requestId(field(data, "requestId")), input: row.input }; }).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentOperation(async () => { await context.runtime.agents.owned(user.id, data.agentId); return context.runtime.agents.execution.insights.replace(context.runtime.agents.execution.operator(data.agentId, user.id), `ui:${data.requestId}`, data.input); });
});

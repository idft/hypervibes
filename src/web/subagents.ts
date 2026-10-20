import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { agentAction, agentUser, field } from "./agents.js";

export const listSubagents = createServerFn({ method: "GET" }).validator((data: unknown) => ({ agentId: field(data, "agentId") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentAction(() => context.runtime.agents.subagents.list(user.id, data.agentId));
});
export const getDashboardSubagents = createServerFn({ method: "GET" }).validator((data: unknown) => ({ agentId: field(data, "agentId") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentAction(() => context.runtime.agents.subagents.dashboard(user.id, data.agentId));
});
export const getSubagent = createServerFn({ method: "GET" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), id: field(data, "id") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentAction(() => context.runtime.agents.subagents.detail(user.id, data.agentId, data.id));
});
export const getSubagentRun = createServerFn({ method: "GET" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), id: field(data, "id"), runId: field(data, "runId") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentAction(() => context.runtime.agents.subagents.runDetail(user.id, data.agentId, data.id, data.runId));
});
export const saveSubagent = createServerFn({ method: "POST" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), id: field(data, "id"), requestId: field(data, "requestId"), templateId: data && typeof data === "object" && Reflect.has(data, "templateId") ? field(data, "templateId") : "", settings: data && typeof data === "object" ? Reflect.get(data, "settings") as unknown : undefined })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentAction(() => data.id ? context.runtime.agents.subagents.update(user.id, data.agentId, data.id, data.settings) : context.runtime.agents.subagents.create(user.id, data.agentId, data.settings, data.requestId, data.templateId));
});
export const saveSubagentSection = createServerFn({ method: "POST" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), id: field(data, "id"), section: field(data, "section"), settings: data && typeof data === "object" ? Reflect.get(data, "settings") as unknown : undefined })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentAction(() => context.runtime.agents.subagents.updateSection(user.id, data.agentId, data.id, data.section, data.settings));
});
export const saveSubagentAssignments = createServerFn({ method: "POST" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), id: field(data, "id"), settings: data && typeof data === "object" ? Reflect.get(data, "settings") as unknown : undefined })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentAction(() => context.runtime.agents.subagents.updateAssignments(user.id, data.agentId, data.id, data.settings));
});
export const saveSubagentIdentity = createServerFn({ method: "POST" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), id: field(data, "id"), name: field(data, "name"), description: field(data, "description") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentAction(() => context.runtime.agents.subagents.updateIdentity(user.id, data.agentId, data.id, data.name, data.description));
});
export const saveSubagentTools = createServerFn({ method: "POST" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), id: field(data, "id"), tools: data && typeof data === "object" ? Reflect.get(data, "tools") as unknown : undefined })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentAction(() => context.runtime.agents.subagents.updateTools(user.id, data.agentId, data.id, data.tools));
});
export const archiveSubagent = createServerFn({ method: "POST" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), id: field(data, "id") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentAction(() => context.runtime.agents.subagents.archive(user.id, data.agentId, data.id));
});
export const unarchiveSubagent = createServerFn({ method: "POST" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), id: field(data, "id") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentAction(() => context.runtime.agents.subagents.unarchive(user.id, data.agentId, data.id));
});
export const runSubagent = createServerFn({ method: "POST" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), id: field(data, "id"), task: field(data, "task"), requestId: field(data, "requestId") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentAction(() => context.runtime.agents.subagents.run(user.id, data.agentId, data.id, data.task, data.requestId));
});
export const stopSubagentRun = createServerFn({ method: "POST" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), runId: field(data, "runId") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentAction(() => context.runtime.agents.subagents.stop(user.id, data.agentId, data.runId));
});

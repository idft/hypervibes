import { isAgentError } from "../agents/agent-input.js";
import { notFound } from "@tanstack/react-router";
import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { agentOperation, agentUser, booleanField, field, optionalNumber } from "./agents.js";

export const listNotifications = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ agentId: field(data, "agentId"), page: optionalNumber(data, "page") ?? 1 }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentOperation(async () => {
      await context.runtime.agents.owned(user.id, data.agentId, true);
      return context.runtime.notifications.list(data.agentId, data.page);
    });
  });
export const getNotification = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ agentId: field(data, "agentId"), id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentOperation(async () => {
      await context.runtime.agents.owned(user.id, data.agentId, true);
      try { return await context.runtime.notifications.get(data.agentId, data.id); }
      catch (error) { if (isAgentError(error) && error.status === 404) throw notFound(); throw error; }
    });
  });
export const getNotificationSettings = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ agentId: field(data, "agentId") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentOperation(async () => {
      await context.runtime.agents.owned(user.id, data.agentId, true);
      return context.runtime.notifications.getSettings(data.agentId);
    });
  });
export const saveNotificationSettings = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ agentId: field(data, "agentId"), enabled: booleanField(data, "enabled"), submissions: booleanField(data, "submissions"), fills: booleanField(data, "fills") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(async () => {
      await context.runtime.agents.owned(user.id, data.agentId);
      return context.runtime.notifications.saveSettings(data.agentId, data);
    });
  });

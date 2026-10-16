import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { AgentError } from "../agents/agent-input.js";
import { agentAction, agentUser, field } from "./agents.js";

export const getProviderSettings = createServerFn({ method: "GET" }).handler(async ({ context }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return context.runtime.providers.settings(user.id);
});
export const getModelChoices = createServerFn({ method: "GET" }).handler(async ({ context }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return context.runtime.providers.choices(user.id);
});
export const getLoginDestination = createServerFn({ method: "GET" }).handler(async ({ context }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return (await context.runtime.providers.choices(user.id)).length ? "/agents" as const : "/providers" as const;
});
export const beginProviderLogin = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ providerId: field(data, "providerId"), type: field(data, "type") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(() => {
      if (data.type !== "api_key" && data.type !== "oauth") throw new AgentError("Invalid authentication method.");
      return context.runtime.providers.begin(user.id, data.providerId, data.type);
    });
  });
export const getProviderLogin = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentAction(async () => context.runtime.providers.status(user.id, data.id));
  });
export const answerProviderLogin = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id"), promptId: field(data, "promptId"), value: field(data, "value") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(async () => context.runtime.providers.answer(user.id, data.id, data.promptId, data.value));
  });
export const cancelProviderLogin = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(() => context.runtime.providers.cancel(user.id, data.id));
  });
export const disconnectProvider = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ providerId: field(data, "providerId") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(() => context.runtime.providers.disconnect(user.id, data.providerId));
  });
export const refreshProviderModels = createServerFn({ method: "POST" }).handler(async ({ context }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentAction(() => context.runtime.providers.refresh(user.id));
});

import { redirect } from "@tanstack/react-router";
import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { AgentError } from "../agents/agent-input.js";
import { currentUser } from "../auth-user.js";
import type { AppRuntime } from "../runtime.js";

export async function agentUser(runtime: AppRuntime, headers: Headers, mutation = false) {
  runtime.status();
  if (mutation && headers.get("origin") !== runtime.origin) throw new AgentError("Invalid request origin", 403);
  const user = await currentUser(runtime, headers);
  if (!user) throw redirect({ to: "/login" });
  return user;
}

function field(data: unknown, key: string): string {
  if (typeof data !== "object" || data === null || !(key in data)) throw new Error("Invalid request");
  const value: unknown = Reflect.get(data, key);
  if (typeof value !== "string") throw new Error("Invalid request");
  return value;
}

export async function agentAction<T>(action: () => Promise<T>) {
  try { return { ok: true as const, data: await action() }; }
  catch (error) {
    if (error instanceof AgentError) return { ok: false as const, error: error.message };
    return { ok: false as const, error: "Agent operation failed. Please try again." };
  }
}

export const listAgents = createServerFn({ method: "GET" }).handler(async ({ context }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return context.runtime.agents.list(user.id);
});

export const createAgent = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ name: field(data, "name"), requestId: field(data, "requestId") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(() => context.runtime.agents.create(user.id, data.name, data.requestId));
  });

export const getAgentChat = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentAction(() => context.runtime.agents.chat(user.id, data.id));
  });

export const sendAgentMessage = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id"), message: field(data, "message"), requestId: field(data, "requestId") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(() => context.runtime.agents.send(user.id, data.id, data.message, data.requestId));
  });

export const retryAgent = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(() => context.runtime.agents.retry(user.id, data.id));
  });

export const stopAgentChat = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(() => context.runtime.agents.stop(user.id, data.id));
  });

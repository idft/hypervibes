import { redirect } from "@tanstack/react-router";
import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { AgentError, isAgentError } from "../agents/agent-input.js";
import { HyperliquidError } from "../hyperliquid/errors.js";
import { currentUser } from "../auth-user.js";
import type { AppRuntime } from "../runtime.js";
import type { AuthenticatedWallet } from "../hyperliquid/service.js";

export async function agentUser(runtime: AppRuntime, headers: Headers, mutation = false) {
  runtime.status();
  if (mutation && headers.get("origin") !== runtime.origin) throw new AgentError("Invalid request origin", 403);
  const user = await currentUser(runtime, headers);
  if (!user) throw redirect({ to: "/login" });
  return user;
}

export function field(data: unknown, key: string): string {
  if (typeof data !== "object" || data === null || !(key in data)) throw new Error("Invalid request");
  const value = Reflect.get(data, key);
  if (typeof value !== "string") throw new Error("Invalid request");
  return value;
}

/** An optional non-negative integer, such as a chat history page token. */
export function optionalNumber(data: unknown, key: string): number | undefined {
  if (typeof data !== "object" || data === null || !(key in data)) return undefined;
  const value = Reflect.get(data, key);
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) throw new Error("Invalid request");
  return value;
}

export async function agentAction<T>(action: () => Promise<T>) {
  try { return { ok: true as const, data: await action() }; }
  catch (error) {
    if (isAgentError(error) || error instanceof HyperliquidError) return { ok: false as const, error: error.message };
    return { ok: false as const, error: "Agent operation failed. Please try again." };
  }
}

export const listAgents = createServerFn({ method: "GET" }).handler(async ({ context }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return context.runtime.agents.list(user.id);
});

/** Validates the selected trading account against fresh Hyperliquid data. */
export async function selectedTradingAccount(runtime: AppRuntime, user: AuthenticatedWallet, selection: string): Promise<string> {
  if (!selection || selection === "none") throw new AgentError("Select an available trading account.");
  const choices = await runtime.hyperliquid.tradingAccountChoices(user);
  if (choices.lookupError) throw new AgentError(choices.lookupError);
  if (!choices.accountModeSupported) throw new AgentError("HyperVibes currently supports Unified Accounts only.");
  if (!choices.apiWalletReady) throw new AgentError("Set up and approve a trading signer first.", 409);
  if (selection === "main") {
    if (choices.mainAssignedTo) throw new AgentError("The main account is already assigned to another agent.");
    return choices.mainAddress;
  }
  const subaccount = choices.subaccounts.find((account) => account.address === selection);
  if (subaccount && !subaccount.assignedTo) return subaccount.address;
  throw new AgentError("Select an available trading account.");
}

export const createAgent = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({
    name: field(data, "name"),
    requestId: field(data, "requestId"),
    accountSelection: field(data, "accountSelection"),
    provider: field(data, "provider"),
    modelId: field(data, "modelId"),
  }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(async () => {
      const model = await context.runtime.providers.validate(user.id, { provider: data.provider, modelId: data.modelId });
      const tradingAccount = await selectedTradingAccount(context.runtime, user, data.accountSelection);
      return context.runtime.agents.create(user.id, data.name, data.requestId, tradingAccount, model);
    });
  });

export const getTradingAccountChoices = createServerFn({ method: "GET" }).handler(async ({ context }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return context.runtime.hyperliquid.tradingAccountChoices(user);
});

export const createAgentSubaccount = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ name: field(data, "name") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(() => context.runtime.hyperliquid.createAgentSubaccount(user, data.name));
  });

export const getAgent = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentAction(() => context.runtime.agents.owned(user.id, data.id));
  });

export const getAgentDashboard = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentAction(() => context.runtime.hyperliquid.agentDashboard(user.id, data.id));
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

export const resetAgentChat = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(() => context.runtime.agents.reset(user.id, data.id));
  });

export const compactAgentChat = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(() => context.runtime.agents.compact(user.id, data.id));
  });

export const getAgentChatHistory = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ id: field(data, "id"), before: optionalNumber(data, "before") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentAction(() => context.runtime.agents.chatHistory(user.id, data.id, data.before));
  });
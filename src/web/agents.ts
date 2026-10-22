import { isNotFound, isRedirect, notFound, redirect } from "@tanstack/react-router";
import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { AgentError, isAgentError, requestId } from "../agents/agent-input.js";
import { IndicatorError } from "../indicators/validation.js";
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

export function booleanField(data: unknown, key: string): boolean {
  if (typeof data !== "object" || data === null || typeof Reflect.get(data, key) !== "boolean") throw new Error("Invalid request");
  return Reflect.get(data, key) as boolean;
}

/** An optional non-negative integer, such as a chat history page token. */
export function optionalNumber(data: unknown, key: string): number | undefined {
  if (typeof data !== "object" || data === null || !(key in data)) return undefined;
  const value = Reflect.get(data, key);
  if (value === undefined) return undefined;
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) throw new Error("Invalid request");
  return value;
}

/** Server functions return domain data or throw a public error. Unknown failures
 * are sanitized before TanStack Start serializes them; redirects and not-found
 * protocol pass through unchanged. */
export async function agentOperation<T>(action: () => Promise<T>): Promise<T> {
  try { return await action(); }
  catch (error) {
    if (error instanceof IndicatorError) throw new AgentError(error.message, error.status);
    if (isAgentError(error) || error instanceof HyperliquidError || isRedirect(error) || isNotFound(error)) throw error;
    throw new AgentError("Agent operation failed. Please try again.", 500);
  }
}

export const listAgents = createServerFn({ method: "GET" }).handler(async ({ context }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentOperation(async () => {
    const agents = await context.runtime.agents.list(user.id);
    return Promise.all(agents.map(async (agent) => ({
      ...agent,
      tradingEnabled: (await context.runtime.agents.risk.get(agent.id)).tradingEnabled,
    })));
  });
});

/** New accounts require fresh exchange validation; an owned creation retry uses its original binding. */
export async function selectedTradingAccount(runtime: AppRuntime, user: AuthenticatedWallet, selection: string, creationRequestId?: string): Promise<string> {
  if (!selection || selection === "none") throw new AgentError("Select an available trading account.");
  if (creationRequestId !== undefined) {
    const id = requestId(creationRequestId);
    const existing = (await runtime.agents.list(user.id)).find((agent) => agent.creationRequestId === id);
    if (existing) {
      const address = (selection === "main" ? user.walletAddress : selection).toLowerCase();
      if (existing.tradingAccountAddress !== address) throw new AgentError("This creation request was already used with different settings", 409);
      return address;
    }
  }
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
    includeDefaultSubagents: booleanField(data, "includeDefaultSubagents"),
    thinkingLevel: typeof data === "object" && data !== null && "thinkingLevel" in data ? field(data, "thinkingLevel") : "",
  }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(async () => {
      const model = await context.runtime.providers.validate(user.id, { provider: data.provider, modelId: data.modelId });
      const tradingAccount = await selectedTradingAccount(context.runtime, user, data.accountSelection, data.requestId);
      return context.runtime.agents.create(user.id, data.name, data.requestId, tradingAccount, model, data.includeDefaultSubagents, data.thinkingLevel);
    });
  });

export const getTradingAccountChoices = createServerFn({ method: "GET" }).handler(async ({ context }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentOperation(() => context.runtime.hyperliquid.tradingAccountChoices(user));
});

export const createAgentSubaccount = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ name: field(data, "name") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(() => context.runtime.hyperliquid.createAgentSubaccount(user, data.name));
  });

export const getAgent = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    try {
      return await agentOperation(() => context.runtime.agents.owned(user.id, data.id, true));
    } catch (error) {
      if (isAgentError(error) && error.status === 404) throw notFound();
      throw error;
    }
  });

export const getAgentDashboard = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentOperation(() => context.runtime.hyperliquid.agentDashboard(user.id, data.id));
  });

export const getAgentChat = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentOperation(() => context.runtime.agents.chat(user.id, data.id));
  });

export const sendAgentMessage = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id"), message: field(data, "message"), requestId: field(data, "requestId"), chart: typeof data === "object" && data !== null && "chart" in data ? data.chart : undefined }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(() => context.runtime.agents.send(user.id, data.id, data.message, data.requestId, data.chart));
  });

export const deleteAgent = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id"), confirmation: field(data, "confirmation") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(async () => {
      const agent = await context.runtime.agents.owned(user.id, data.id, true);
      if (data.confirmation !== agent.name) throw new AgentError("Type the agent name to confirm deletion.");
      await context.runtime.agents.delete(user.id, agent.id);
      await context.runtime.gateway.removed(agent.id);
      return { deleted: true };
    });
  });

export const retryAgent = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(() => context.runtime.agents.retry(user.id, data.id));
  });

export const stopAgentChat = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(() => context.runtime.agents.stop(user.id, data.id));
  });

export const resetAgentChat = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(() => context.runtime.agents.reset(user.id, data.id));
  });

export const compactAgentChat = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(() => context.runtime.agents.compact(user.id, data.id));
  });

export const getAgentChatHistory = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ id: field(data, "id"), before: optionalNumber(data, "before") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentOperation(() => context.runtime.agents.chatHistory(user.id, data.id, data.before));
  });
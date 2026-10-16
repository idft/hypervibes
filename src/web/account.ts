import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { HyperliquidError } from "../hyperliquid/errors.js";
import { agentUser } from "./agents.js";

async function accountAction<T>(action: () => Promise<T>) {
  try { return { ok: true as const, data: await action() }; }
  catch (error) {
    if (error instanceof HyperliquidError) return { ok: false as const, error: error.message };
    return { ok: false as const, error: "Account operation failed. Please try again." };
  }
}

function field(data: unknown, key: string): string {
  if (typeof data !== "object" || data === null || !(key in data)) throw new Error("Invalid request");
  const value = Reflect.get(data, key);
  if (typeof value !== "string") throw new Error("Invalid request");
  return value;
}

export const getAccount = createServerFn({ method: "GET" }).handler(async ({ context }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return context.runtime.hyperliquid.accountView(user);
});

export const setupApiWallet = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({
    source: field(data, "source"),
    privateKey: "privateKey" in (data as object) ? field(data, "privateKey") : "",
  }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return accountAction(() => context.runtime.hyperliquid.storeApiWallet(user.id, data.source === "import" ? "import" : "generate", data.privateKey));
  });

export const approveApiWallet = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ request: data }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return accountAction(() => context.runtime.hyperliquid.approveApiWallet(user, data.request));
  });

export const approveBuilderFee = createServerFn({ method: "POST" })
  .validator((data: unknown) => {
    if (typeof data !== "object" || data === null || typeof Reflect.get(data, "feeBps") !== "number") throw new Error("Invalid request");
    return { request: data, feeBps: Reflect.get(data, "feeBps") as number };
  })
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return accountAction(() => context.runtime.hyperliquid.approveBuilderFee(user, data.request, data.feeBps));
  });

export const cancelBuilderFee = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ request: data }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return accountAction(() => context.runtime.hyperliquid.cancelBuilderFee(user, data.request));
  });
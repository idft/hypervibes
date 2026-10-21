import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { agentOperation, agentUser } from "./agents.js";

function field(data: unknown, key: string): string {
  if (typeof data !== "object" || data === null || !(key in data)) throw new Error("Invalid request");
  const value = Reflect.get(data, key);
  if (typeof value !== "string") throw new Error("Invalid request");
  return value;
}

export const getAccount = createServerFn({ method: "GET" }).handler(async ({ context }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentOperation(() => context.runtime.hyperliquid.accountView(user));
});

export const setupApiWallet = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({
    source: field(data, "source"),
    privateKey: "privateKey" in (data as object) ? field(data, "privateKey") : "",
  }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(() => context.runtime.hyperliquid.storeApiWallet(user.id, data.source === "import" ? "import" : "generate", data.privateKey));
  });

export const approveApiWallet = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ request: data }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(() => context.runtime.hyperliquid.approveApiWallet(user, data.request));
  });

export const approveBuilderFee = createServerFn({ method: "POST" })
  .validator((data: unknown) => {
    if (typeof data !== "object" || data === null || typeof Reflect.get(data, "feeBps") !== "number") throw new Error("Invalid request");
    return { request: data, feeBps: Reflect.get(data, "feeBps") as number };
  })
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(() => context.runtime.hyperliquid.approveBuilderFee(user, data.request, data.feeBps));
  });

export const cancelBuilderFee = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ request: data }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(() => context.runtime.hyperliquid.cancelBuilderFee(user, data.request));
  });
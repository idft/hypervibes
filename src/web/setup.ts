import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { agentUser } from "./agents.js";

export const getSetupState = createServerFn({ method: "GET" }).handler(async ({ context }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  const agents = await context.runtime.agents.list(user.id);
  if (agents.length > 0) return { agentCount: agents.length, providerConnected: false, apiKeyApproved: false, builderFeeApproved: false };
  const [providers, account] = await Promise.all([
    context.runtime.providers.settings(user.id),
    context.runtime.hyperliquid.accountView(user),
  ]);
  return {
    agentCount: 0,
    providerConnected: providers.providers.some((provider) => provider.connected),
    apiKeyApproved: account.apiWallet?.state === "approved",
    builderFeeApproved: account.builderFee.approved,
  };
});

import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { agentAction, agentUser, field } from "./agents.js";

/** Risk management settings are the only host-enforced trading rules. */
export const getRiskSettings = createServerFn({ method: "GET" }).validator((data: unknown) => ({ agentId: field(data, "agentId") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentAction(async () => {
    await context.runtime.agents.owned(user.id, data.agentId, true);
    return context.runtime.agents.risk.get(data.agentId);
  });
});
export const saveRiskSettings = createServerFn({ method: "POST" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), settings: data && typeof data === "object" ? Reflect.get(data, "settings") as unknown : undefined })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentAction(async () => {
    await context.runtime.agents.owned(user.id, data.agentId);
    return context.runtime.agents.risk.save(data.agentId, data.settings);
  });
});
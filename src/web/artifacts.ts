import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { agentOperation, agentUser, field, optionalNumber } from "./agents.js";

export const readArtifact = createServerFn({ method: "GET" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), id: field(data, "id"), offset: optionalNumber(data, "offset") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentOperation(async () => { await context.runtime.agents.owned(user.id, data.agentId); return context.runtime.agents.research.artifacts.read(data.agentId, data.id, data.offset); });
});
export const changeArtifact = createServerFn({ method: "POST" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), id: field(data, "id"), action: field(data, "action") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentOperation(async () => {
    await context.runtime.agents.owned(user.id, data.agentId);
    const service = context.runtime.agents.research.artifacts;
    if (data.action === "delete") await service.remove(data.agentId, data.id);
    else if (data.action === "retain" || data.action === "expire") await service.retention(data.agentId, data.id, data.action === "retain");
    else throw new Error("Invalid artifact action");
    return { updated: true };
  });
});

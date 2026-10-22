import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { AgentError } from "../agents/agent-input.js";
import { fetchBotUsername } from "../gateway/telegram/transport.js";
import { deleteGateway, getGateway, saveGatewayToken } from "../gateway/gateway-store.js";
import { agentOperation, agentUser, field } from "./agents.js";

/** All server functions target the Telegram gateway, the platform this application currently bridges. */
const type = "telegram" as const;

export const getGatewayStatus = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentOperation(async () => {
      await context.runtime.agents.owned(user.id, data.id, true);
      return context.runtime.gateway.status(data.id);
    });
  });

export const addGatewayToken = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id"), token: field(data, "token") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(async () => {
      await context.runtime.agents.owned(user.id, data.id);
      const key = context.runtime.gateway.requireKey();
      if (await getGateway(context.runtime.db, data.id, type)) throw new AgentError("Disconnect the existing Telegram bot before adding a new one.", 409);
      const token = data.token.trim();
      if (!token) throw new AgentError("Enter the bot token from @BotFather.");
      // Validate before persisting: a rejected token must not replace the visible configuration.
      let botUsername: string;
      try { botUsername = await fetchBotUsername(token); }
      catch { throw new AgentError("Telegram rejected this bot token. Check the token from @BotFather."); }
      await saveGatewayToken(context.runtime.db, key, data.id, type, token, botUsername);
      await context.runtime.gateway.ensure(data.id);
      return context.runtime.gateway.status(data.id);
    });
  });

export const startGatewayLink = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(async () => {
      await context.runtime.agents.owned(user.id, data.id);
      return { link: await context.runtime.gateway.startLink(data.id, user.id) };
    });
  });

export const disconnectGateway = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentOperation(async () => {
      await context.runtime.agents.owned(user.id, data.id);
      await deleteGateway(context.runtime.db, data.id, type);
      await context.runtime.gateway.removed(data.id);
      return context.runtime.gateway.status(data.id);
    });
  });
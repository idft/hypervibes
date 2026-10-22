import type { AgentBridge } from "../../agents/agent-service.js";
import type { GatewayRow } from "../../db/gateway-schema.js";
import { bridgeErrorText, submitChat, type BridgeMessage, type BridgeWorkspace, type GatewayConnector, type GatewaySender } from "../handlers.js";
import { grammyEndpoint } from "./transport.js";

interface ParsedCommand { name: string; arg: string }

/** Splits a `/name[@bot] args` message; undefined for plain text. */
function parseCommand(text: string): ParsedCommand | undefined {
  const match = /^\/([a-zA-Z]+)(?:@[a-zA-Z0-9_]+)?(?:\s+([\s\S]*))?$/.exec(text);
  const name = match?.[1]?.toLowerCase();
  if (!name) return undefined;
  return { name, arg: (match?.[2] ?? "").trim() };
}

/** The `/start <token>` payload of a link deep link; undefined for other messages. */
function extractStartPayload(text: string): string | undefined {
  const command = parseCommand(text);
  if (command?.name !== "start") return undefined;
  return command.arg || undefined;
}

const HELP_TEXT = [
  "Commands:",
  "/reset — start a fresh conversation",
  "/stop — stop the active turn",
  "/compact — compact the conversation",
  "/help — show this help",
  "Everything else goes to this Agent's main conversation.",
].join("\n");

async function consumeStart(agentId: string, message: BridgeMessage, token: string, workspace: BridgeWorkspace, sender: GatewaySender): Promise<void> {
  const result = workspace.consumeLink(token);
  if (result.status === "unknown") {
    await sender.sendText(message.chatId, "This link token is unknown or has expired. Start a new link from the web UI.");
    return;
  }
  if (result.status === "expired") {
    await sender.sendText(message.chatId, "This link token has expired. Start a new link from the web UI.");
    return;
  }
  if (result.agentId !== agentId) {
    await sender.sendText(message.chatId, "This link token belongs to a different agent.");
    return;
  }
  const bound = await workspace.bindChat(agentId, message.chatId, message.fromUsername);
  if (!bound) {
    await sender.sendText(message.chatId, "This Telegram gateway is no longer configured. Start a new link from the web UI.");
    return;
  }
  await sender.sendText(message.chatId, "Telegram chat linked. You can now use this chat with HyperVibes.");
}

async function handleCommand(agentId: string, chatId: number, command: ParsedCommand, workspace: BridgeWorkspace, sender: GatewaySender): Promise<void> {
  if (command.name === "help") {
    await sender.sendText(chatId, HELP_TEXT);
    return;
  }
  let handle: AgentBridge;
  try { handle = await workspace.bridge(agentId); }
  catch (error) { await sender.sendText(chatId, bridgeErrorText(error)); return; }
  try {
    if (command.name === "stop") {
      await handle.stop();
      await sender.sendText(chatId, "Active turn stopped.");
    } else if (command.name === "reset") {
      await handle.reset();
      await sender.sendText(chatId, "Started a fresh conversation.");
    } else if (command.name === "compact") {
      const result = await handle.compact();
      await sender.sendText(chatId, result.message ?? "Conversation compacted.");
    } else {
      await sender.sendText(chatId, HELP_TEXT);
    }
  } catch (error) {
    workspace.logger?.warn("Telegram command failed", error);
    await sender.sendText(chatId, bridgeErrorText(error));
  }
}

async function handleTelegramMessage(agentId: string, message: BridgeMessage, workspace: BridgeWorkspace, sender: GatewaySender): Promise<void> {
  const text = message.text.trim();
  const startToken = extractStartPayload(text);
  if (startToken) {
    await consumeStart(agentId, message, startToken, workspace, sender);
    return;
  }
  const command = parseCommand(text);
  if (command?.name === "start") {
    // `/start` without a payload: guide the operator toward the link flow.
    await sender.sendText(message.chatId, "To link this chat, open your Agent's settings page and start a Telegram link.");
    return;
  }
  const gateway = await workspace.loadGateway(agentId);
  // Only the chat that completed the link flow may talk to the Agent.
  if (!gateway || gateway.chatId === null || gateway.chatId !== message.chatId) {
    workspace.logger?.warn(`Telegram message from unauthorized chat ${message.chatId} ignored`);
    return;
  }
  if (command) {
    await handleCommand(agentId, message.chatId, command, workspace, sender);
    return;
  }
  if (text) await submitChat(agentId, message.chatId, text, workspace, sender);
}

/** The Telegram platform: deep links, commands, and the chat authorization model. */
export const telegramConnector: GatewayConnector = {
  type: "telegram",
  label: "Telegram",
  createEndpoint: grammyEndpoint,
  handle: handleTelegramMessage,
  linkUrl: (row: GatewayRow, token: string) => `https://t.me/${row.botUsername}?start=${token}`,
};
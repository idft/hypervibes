import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import type { JsonValue } from "@earendil-works/chord";
import type { Submission } from "@earendil-works/pi-durable";
import { randomUUID } from "node:crypto";
import { isAgentError } from "../agents/agent-input.js";
import type { AgentBridge } from "../agents/agent-service.js";
import type { GatewayRow, GatewayType } from "../db/gateway-schema.js";

/** A deep-link token is valid for ten minutes, matching the legacy gateway. */
export const PENDING_LINK_TTL_MS = 10 * 60 * 1000;
/** How long the bridge forwards one turn's reply before giving up on it. */
export const REPLY_TIMEOUT_MS = 30 * 60 * 1000;
/** The typing action expires after ~5 seconds and is refreshed while a turn runs. */
const TYPING_REFRESH_MS = 4500;

/** Minimal logger shape the bridge needs; Fastify child loggers satisfy it. */
export interface BridgeLogger {
  warn(message: string, ...args: unknown[]): void;
  error(message: string, ...args: unknown[]): void;
}

/** Sends on behalf of the bridge. */
export interface GatewaySender {
  sendText(chatId: number, text: string): Promise<void>;
  sendTyping(chatId: number): Promise<void>;
}

/** One running gateway endpoint: message intake plus sending. */
export interface GatewayEndpoint extends GatewaySender {
  /**
   * Resolves when polling stops; rejects on an error such as a revoked token.
   * `onStarted` fires once the platform accepted the connection, so a delayed
   * retry can clear a previously recorded failure.
   */
  start(onStarted?: () => void): Promise<void>;
  stop(): Promise<void>;
}

/** A gateway message reduced to the fields the bridge uses. */
export interface BridgeMessage {
  chatId: number;
  text: string;
  fromUsername?: string;
}

/** An in-memory link started from the settings page and consumed by the platform's link command. */
export interface PendingLink {
  token: string;
  agentId: string;
  userId: string;
  expiresAt: number;
}

/** Link lookups and agent access; implemented by the bridge service and fakes in tests. */
export interface BridgeWorkspace {
  loadGateway(agentId: string): Promise<GatewayRow | undefined>;
  bindChat(agentId: string, chatId: number, chatUsername: string | undefined): Promise<boolean>;
  /** Consumes a pending link token if it is valid, or reports why not. */
  consumeLink(token: string): { status: "ok"; agentId: string } | { status: "expired" } | { status: "unknown" };
  bridge(agentId: string): Promise<AgentBridge>;
  logger?: BridgeLogger;
  /** Test seam: overrides the reply forwarding timeout. */
  replyTimeoutMs?: number;
}

/** One gateway platform: transport, message routing, and deep links. */
export interface GatewayConnector {
  readonly type: GatewayType;
  /** Platform label for operator-facing messages. */
  readonly label: string;
  /** Builds the running endpoint for a decrypted bot token. */
  createEndpoint(token: string, onMessage: (message: BridgeMessage) => void, logger?: BridgeLogger): GatewayEndpoint;
  /** Routes one message; must not throw, the bridge logs instead. */
  handle(agentId: string, message: BridgeMessage, workspace: BridgeWorkspace, sender: GatewaySender): Promise<void>;
  /** The platform URL a pending link token directs the operator to. */
  linkUrl(row: GatewayRow, token: string): string;
}

/** Chunked for the platform's message limit; split by code points so emoji survive. */
export function chunkText(text: string, max: number): string[] {
  const characters = [...text];
  const chunks: string[] = [];
  for (let index = 0; index < characters.length; index += max) chunks.push(characters.slice(index, index + max).join(""));
  return chunks.length ? chunks : [""];
}

/** A public, bounded error message; AgentError and plain reasons carry operator-facing text. */
export function bridgeErrorText(error: unknown): string {
  const detail = typeof error === "string" ? error : isAgentError(error) || error instanceof Error ? error.message : "Unknown error";
  return `I couldn't process that message: ${[...detail].slice(0, 1000).join("")}`;
}

/**
 * The operator-facing text of a terminal submission. `reason` is a machine code such as `model_error`;
 * `detail` carries the provider's actual message, which is what an operator can act on.
 */
export function submissionErrorText(settled: { readonly reason: string; readonly detail?: JsonValue }): string {
  const detail = typeof settled.detail === "string" ? settled.detail.trim() : "";
  return detail || settled.reason;
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms);
    signal.addEventListener("abort", () => { clearTimeout(timer); resolve(); }, { once: true });
  });
}

/** Refreshes the typing action while the wrapped operation runs; never rejects on its own. */
function withTyping(sender: GatewaySender, chatId: number, operation: () => Promise<void>): Promise<void> {
  const stop = new AbortController();
  void (async () => {
    while (!stop.signal.aborted) {
      await sender.sendTyping(chatId).catch(() => undefined);
      await sleep(TYPING_REFRESH_MS, stop.signal);
    }
  })();
  return operation().finally(() => { stop.abort(); });
}

/** Forwards the assistant reply once the submitted turn settles. */
export async function forwardReply(workspace: BridgeWorkspace, sender: GatewaySender, chatId: number, handle: AgentBridge, submission: Submission): Promise<void> {
  const timeout = workspace.replyTimeoutMs ?? REPLY_TIMEOUT_MS;
  const expired = new Promise<undefined>((resolve) => AbortSignal.timeout(timeout).addEventListener("abort", () => resolve(undefined), { once: true }));
  await withTyping(sender, chatId, async () => {
    const waiting = submission.wait(context);
    // The race may already be decided by the timeout when this rejects later.
    waiting.catch(() => undefined);
    const settled = await Promise.race([waiting, expired]);
    if (settled === undefined) {
      await sender.sendText(chatId, "This response is taking longer than expected. The turn is still running; continue in the web dashboard.");
      return;
    }
    if (settled.status === "unanswered" || settled.entry === undefined || settled.answer === undefined) {
      await sender.sendText(chatId, bridgeErrorText(settled.status === "unanswered" ? submissionErrorText(settled) : "The turn produced no reply"));
      return;
    }
    const replies = await handle.replyText(settled.entry, settled.answer);
    for (const reply of replies) await sender.sendText(chatId, reply);
  });
}

/** Submits plain chat text to the Agent's main conversation and forwards the reply in the background. */
export async function submitChat(agentId: string, chatId: number, text: string, workspace: BridgeWorkspace, sender: GatewaySender): Promise<void> {
  let handle: AgentBridge;
  let submission: Submission;
  try {
    handle = await workspace.bridge(agentId);
    submission = await handle.submit(text, randomUUID());
  } catch (error) {
    await sender.sendText(chatId, bridgeErrorText(error));
    return;
  }
  const status = await submission.status(context).catch(() => undefined);
  if (status?.status === "queued") await sender.sendText(chatId, "Message queued. It will send when the current turn finishes.");
  // The turn runs in the background so the next message is picked up immediately,
  // matching the legacy gateway's fire-and-forget reply forwarder.
  void forwardReply(workspace, sender, chatId, handle, submission).catch((error) => {
    workspace.logger?.warn("Gateway reply forwarding failed", error);
  });
}
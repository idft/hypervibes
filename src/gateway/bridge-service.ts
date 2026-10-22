import { sendTelegramNotification } from "./telegram/transport.js";
import { NotificationDeliveryError, type NotificationTarget } from "../notifications/delivery.js";
import { randomUUID } from "node:crypto";
import type { FastifyBaseLogger } from "fastify";
import { AgentError } from "../agents/agent-input.js";
import type { AgentService } from "../agents/agent-service.js";
import type { EncryptionKeySetting } from "../config.js";
import type { ApplicationDatabase } from "../db/shared-database.js";
import { bindGatewayChat, decryptGatewayToken, gatewayStatus, getGateway, listGateways, type GatewayStatus } from "./gateway-store.js";
import type { GatewayRow } from "../db/gateway-schema.js";
import { PENDING_LINK_TTL_MS, type BridgeMessage, type BridgeWorkspace, type GatewayConnector, type GatewayEndpoint, type PendingLink } from "./handlers.js";

export interface GatewayBridgeOptions {
  logger?: FastifyBaseLogger;
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Owns one gateway platform's endpoints. Event-driven lifecycle: configured
 * gateways start once at startup, and later changes flow through `ensure()`
 * and `removed()` from the settings mutations.
 */
export class GatewayBridge {
  private readonly endpoints = new Map<string, GatewayEndpoint>();
  private readonly pending = new Map<string, PendingLink>();
  private readonly errors = new Map<string, string>();
  private started = false;
  private stopping = false;
  private closing?: Promise<void>;

  constructor(private readonly db: ApplicationDatabase, private readonly agents: AgentService, private readonly key: EncryptionKeySetting | undefined, private readonly connector: GatewayConnector, private readonly options: GatewayBridgeOptions = {}) {}

  get type() { return this.connector.type; }

  /** The active encryption key; gateway credentials require one on the server. */
  requireKey(): EncryptionKeySetting {
    if (!this.key) throw new AgentError(`${this.connector.label} gateways require AGENTS_ENCRYPTION_KEY on the server.`, 503);
    return this.key;
  }

  /** Loads this platform's configured gateways once and starts their endpoints. */
  start() {
    if (this.started || this.stopping) return;
    this.started = true;
    void listGateways(this.db, this.type).then((rows) => Promise.all(rows.map((row) => this.startEndpoint(row))))
      .catch((error) => { if (!this.stopping) this.options.logger?.error(`Unable to start ${this.connector.label} bridge: ${describe(error)}`); });
  }

  /** Starts the endpoint after a gateway row was created, if it is not running yet. */
  async ensure(agentId: string) {
    if (this.stopping || this.endpoints.has(agentId)) return;
    const row = await getGateway(this.db, agentId, this.type);
    if (row) await this.startEndpoint(row);
  }

  /** Stops the endpoint and drops pending links: disconnect or agent deletion. */
  async removed(agentId: string) {
    for (const [token, link] of this.pending) if (link.agentId === agentId) this.pending.delete(token);
    this.errors.delete(agentId);
    const endpoint = this.endpoints.get(agentId);
    if (!endpoint) return;
    this.endpoints.delete(agentId);
    try { await endpoint.stop(); }
    catch (error) { this.options.logger?.warn(`${this.connector.label} endpoint stop failed: ${describe(error)}`); }
  }

  /** Creates or returns the owner's pending link token and its deep-link URL. */
  async startLink(agentId: string, userId: string): Promise<string> {
    const row = await getGateway(this.db, agentId, this.type);
    if (!row) throw new AgentError(`Add a ${this.connector.label} bot token first.`, 400);
    let existing: PendingLink | undefined;
    for (const [token, link] of this.pending) {
      if (link.agentId !== agentId) continue;
      if (link.userId !== userId || link.expiresAt <= Date.now()) this.pending.delete(token);
      else existing = { ...link, token };
    }
    const link = existing ?? { token: randomUUID(), agentId, userId, expiresAt: Date.now() + PENDING_LINK_TTL_MS };
    this.pending.set(link.token, link);
    return this.connector.linkUrl(row, link.token);
  }

  /** A queued notification is bound to its original destination, never a newly linked chat. */
  async sendNotification(target: NotificationTarget, text: string, signal: AbortSignal): Promise<number> {
    if (this.stopping) throw new NotificationDeliveryError("Gateway is shutting down");
    const row = await getGateway(this.db, target.agentId, this.type);
    if (!row || row.chatId !== target.chatId || row.botTokenHash !== target.botTokenHash) throw new NotificationDeliveryError("Telegram destination changed", true, undefined, true);
    const token = decryptGatewayToken(this.key, row);
    if (!token) throw new NotificationDeliveryError("Telegram token cannot be decrypted; reconnect Telegram in settings", true);
    return sendTelegramNotification(token, target.chatId, text, signal);
  }

  /** Render-safe gateway state for the settings page. */
  async status(agentId: string): Promise<GatewayStatus> {
    const row = await getGateway(this.db, agentId, this.type);
    return gatewayStatus(row, this.errors.get(agentId));
  }

  /** Stops every endpoint and drops all link state. */
  close(): Promise<void> {
    return this.closing ??= (async () => {
      this.stopping = true;
      const endpoints = [...this.endpoints.values()];
      this.endpoints.clear();
      this.pending.clear();
      await Promise.allSettled(endpoints.map((endpoint) => endpoint.stop()));
    })();
  }

  private async startEndpoint(row: GatewayRow) {
    if (this.stopping || this.endpoints.has(row.agentId)) return;
    const token = decryptGatewayToken(this.key, row);
    if (!token) {
      this.errors.set(row.agentId, "The bot token cannot be decrypted with the active encryption key. Re-add the bot token in settings.");
      this.options.logger?.warn(`Bot token for agent ${row.agentId} cannot be decrypted; endpoint not started`);
      return;
    }
    const endpoint = this.connector.createEndpoint(token, (message) => { void this.receive(row.agentId, message); }, this.options.logger);
    this.endpoints.set(row.agentId, endpoint);
    this.errors.delete(row.agentId);
    void endpoint.start().then(
      () => {
        if (this.endpoints.get(row.agentId) === endpoint) this.endpoints.delete(row.agentId);
      },
      (error: unknown) => {
        if (this.endpoints.get(row.agentId) === endpoint) this.endpoints.delete(row.agentId);
        this.errors.set(row.agentId, describe(error));
        this.options.logger?.warn(`${this.connector.label} endpoint for agent ${row.agentId} stopped: ${describe(error)}`);
      },
    );
  }

  private workspace(): BridgeWorkspace {
    return {
      loadGateway: (agentId) => getGateway(this.db, agentId, this.type),
      bindChat: (agentId, chatId, chatUsername) => bindGatewayChat(this.db, agentId, this.type, chatId, chatUsername),
      consumeLink: (token) => this.consumeLink(token),
      bridge: (agentId) => this.agents.bridge(agentId),
      logger: this.options.logger,
    };
  }

  /** One-shot link consumption; the token dies with its first use or its expiry. */
  private consumeLink(token: string): { status: "ok"; agentId: string } | { status: "expired" } | { status: "unknown" } {
    const link = this.pending.get(token);
    if (!link) return { status: "unknown" };
    this.pending.delete(token);
    if (link.expiresAt <= Date.now()) return { status: "expired" };
    return { status: "ok", agentId: link.agentId };
  }

  private async receive(agentId: string, message: BridgeMessage) {
    const sender = this.endpoints.get(agentId);
    if (!sender) return; // The endpoint stopped; a restarted one will pick up future updates.
    try { await this.connector.handle(agentId, message, this.workspace(), sender); }
    catch (error) { this.options.logger?.warn(`${this.connector.label} message handling for agent ${agentId} failed: ${describe(error)}`); }
  }
}
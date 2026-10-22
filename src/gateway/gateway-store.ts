import { and, eq } from "drizzle-orm";
import { createHash } from "node:crypto";
import { AgentError } from "../agents/agent-input.js";
import type { EncryptionKeySetting } from "../config.js";
import { hvGateways, type GatewayRow, type GatewayType } from "../db/gateway-schema.js";
import type { ApplicationDatabase } from "../db/shared-database.js";
import { decryptSecret, encryptSecret } from "../hyperliquid/crypto.js";

/** Render-safe projection for settings and API responses; never contains token material. */
export interface GatewayStatus {
  hasToken: boolean;
  connected: boolean;
  botUsername: string | null;
  chatId: number | null;
  chatUsername: string | null;
  /** The endpoint's last fatal error, such as a revoked token; absent while healthy. */
  error?: string;
}

/** Deterministic token fingerprint backing the cross-Agent uniqueness constraint. */
export function hashBotToken(token: string): string {
  return createHash("sha256").update(token).digest("hex");
}

export async function getGateway(db: ApplicationDatabase, agentId: string, type: GatewayType): Promise<GatewayRow | undefined> {
  return db.select().from(hvGateways).where(and(eq(hvGateways.agentId, agentId), eq(hvGateways.type, type))).get();
}

/** Every configured gateway of one type; token presence means the endpoint should run. */
export async function listGateways(db: ApplicationDatabase, type: GatewayType): Promise<GatewayRow[]> {
  return db.select().from(hvGateways).where(eq(hvGateways.type, type)).all();
}

/** The bot token of one configured gateway; undefined when the encryption key is missing or changed. */
export function decryptGatewayToken(key: EncryptionKeySetting | undefined, row: GatewayRow): string | undefined {
  if (!key || row.keyId !== key.id) return undefined;
  try { return decryptSecret(key, row.botTokenCiphertext); }
  catch { return undefined; }
}

function uniqueTokenViolation(error: unknown): boolean {
  let current: unknown = error;
  for (let depth = 0; current !== null && typeof current === "object" && depth < 5; depth++) {
    const message = (current as { message?: unknown }).message;
    if (typeof message === "string" && message.includes("UNIQUE constraint failed") && message.includes("bot_token_hash")) return true;
    current = (current as { cause?: unknown }).cause;
  }
  return false;
}

/** Stores a newly validated bot token; the token hash stops two Agents sharing one bot. */
export async function saveGatewayToken(db: ApplicationDatabase, key: EncryptionKeySetting, agentId: string, type: GatewayType, token: string, botUsername: string): Promise<void> {
  const values = {
    botTokenCiphertext: Buffer.from(encryptSecret(key, token)),
    botTokenHash: hashBotToken(token),
    keyId: key.id,
    botUsername,
    updatedAt: Date.now(),
  };
  try {
    await db.insert(hvGateways).values({ agentId, type, ...values }).run();
  } catch (error) {
    if (uniqueTokenViolation(error)) throw new AgentError("This bot is already linked to another Agent.", 409);
    throw error;
  }
}

/** Persists the chat that completed the link flow; false when no gateway row exists. */
export async function bindGatewayChat(db: ApplicationDatabase, agentId: string, type: GatewayType, chatId: number, chatUsername: string | undefined): Promise<boolean> {
  const row = await getGateway(db, agentId, type);
  if (!row) return false;
  await db.update(hvGateways).set({ chatId, chatUsername, updatedAt: Date.now() }).where(and(eq(hvGateways.agentId, agentId), eq(hvGateways.type, type))).run();
  return true;
}

/** Removes the bot token and binding; false when no gateway row exists. */
export async function deleteGateway(db: ApplicationDatabase, agentId: string, type: GatewayType): Promise<boolean> {
  const row = await getGateway(db, agentId, type);
  if (!row) return false;
  await db.delete(hvGateways).where(and(eq(hvGateways.agentId, agentId), eq(hvGateways.type, type))).run();
  return true;
}

/** Status view of a gateway row, extended with an optional runtime endpoint error. */
export function gatewayStatus(row: GatewayRow | undefined, error?: string): GatewayStatus {
  if (!row) return { hasToken: false, connected: false, botUsername: null, chatId: null, chatUsername: null };
  return { hasToken: true, connected: row.chatId !== null, botUsername: row.botUsername, chatId: row.chatId, chatUsername: row.chatUsername, ...(error ? { error } : {}) };
}
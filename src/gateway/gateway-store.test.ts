import { eq } from "drizzle-orm";
import { createHash, randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { EncryptionKeySetting } from "../config.js";
import { hvAgents } from "../db/agent-schema.js";
import { hvUsers } from "../db/auth-schema.js";
import { openDatabase } from "../db/database.js";
import { hvGateways, type GatewayRow } from "../db/gateway-schema.js";
import { migrateApplication } from "../db/migrations.js";
import { applicationDatabase, SharedDatabase } from "../db/shared-database.js";
import { bindGatewayChat, decryptGatewayToken, deleteGateway, gatewayStatus, getGateway, hashBotToken, listGateways, saveGatewayToken } from "./gateway-store.js";

const key: EncryptionKeySetting = { id: "primary", bytes: new Uint8Array(createHash("sha256").update("test-gateway-key").digest()) };
const otherKey: EncryptionKeySetting = { id: "rotated", bytes: key.bytes };

describe("gateway storage", () => {
  let directory: string;
  let db: ReturnType<typeof applicationDatabase>;
  let database: SharedDatabase;
  beforeEach(() => {
    directory = mkdtempSync(join(tmpdir(), "hv-gateways-"));
    const connection = openDatabase(join(directory, "test.sqlite"));
    migrateApplication(connection);
    database = new SharedDatabase(connection);
    db = applicationDatabase(database);
    void db.insert(hvUsers).values({ id: "owner", name: "Owner", email: "owner@example.test" }).run();
  });
  afterEach(async () => {
    await database.close();
    rmSync(directory, { recursive: true, force: true });
  });

  async function seedAgent(id = randomUUID()) {
    const now = Date.now();
    await db.insert(hvAgents).values({ id, userId: "owner", name: "Agent", creationRequestId: randomUUID(), createdAt: now, updatedAt: now }).run();
    return id;
  }

  it("stores an encrypted token with a deterministic hash and projects a safe status", async () => {
    const agentId = await seedAgent();
    await saveGatewayToken(db, key, agentId, "telegram", "bot-token-value", "my_bot");
    const row = (await getGateway(db, agentId, "telegram"))!;
    expect(row.keyId).toBe("primary");
    expect(row.botTokenHash).toBe(hashBotToken("bot-token-value"));
    expect(row.botTokenCiphertext.toString("binary")).not.toContain("bot-token-value");
    expect(row.chatId).toBeNull();
    expect(gatewayStatus(row)).toEqual({ hasToken: true, connected: false, botUsername: "my_bot", chatId: null, chatUsername: null });
    expect(decryptGatewayToken(key, row)).toBe("bot-token-value");
  });

  it("rejects the same bot token for a second agent but allows reuse after disconnect", async () => {
    const first = await seedAgent();
    const second = await seedAgent();
    await saveGatewayToken(db, key, first, "telegram", "shared-token", "my_bot");
    await expect(saveGatewayToken(db, key, second, "telegram", "shared-token", "my_bot")).rejects.toThrow("already linked to another Agent");
    expect(await getGateway(db, second, "telegram")).toBeUndefined();
    expect(await deleteGateway(db, first, "telegram")).toBe(true);
    await saveGatewayToken(db, key, second, "telegram", "shared-token", "my_bot");
    expect(decryptGatewayToken(key, (await getGateway(db, second, "telegram"))!)).toBe("shared-token");
  });

  it("keeps the token unreadable under a rotated encryption key", async () => {
    const agentId = await seedAgent();
    await saveGatewayToken(db, key, agentId, "telegram", "bot-token-value", "my_bot");
    const row = (await getGateway(db, agentId, "telegram"))!;
    expect(decryptGatewayToken(key, row)).toBe("bot-token-value");
    expect(decryptGatewayToken(otherKey, row)).toBeUndefined();
    expect(decryptGatewayToken(undefined, row)).toBeUndefined();
  });

  it("binds the chat that completed the link and reports it connected", async () => {
    const agentId = await seedAgent();
    expect(await bindGatewayChat(db, agentId, "telegram", -100123, "operator")).toBe(false);
    await saveGatewayToken(db, key, agentId, "telegram", "bot-token-value", "my_bot");
    expect(await bindGatewayChat(db, agentId, "telegram", -100123, "operator")).toBe(true);
    const status = gatewayStatus(await getGateway(db, agentId, "telegram"));
    expect(status).toEqual({ hasToken: true, connected: true, botUsername: "my_bot", chatId: -100123, chatUsername: "operator" });
    expect(await listGateways(db, "telegram")).toHaveLength(1);
  });

  it("removes the whole gateway on delete and leaves other agents alone", async () => {
    const first = await seedAgent();
    const second = await seedAgent();
    await saveGatewayToken(db, key, first, "telegram", "token-one", "one_bot");
    await saveGatewayToken(db, key, second, "telegram", "token-two", "two_bot");
    await bindGatewayChat(db, first, "telegram", 42, "operator");
    expect(await deleteGateway(db, first, "telegram")).toBe(true);
    expect(await deleteGateway(db, first, "telegram")).toBe(false);
    expect(await getGateway(db, first, "telegram")).toBeUndefined();
    expect(await listGateways(db, "telegram")).toHaveLength(1);
  });

  it("cascades gateway rows when the owning agent is deleted", async () => {
    const agentId = await seedAgent();
    await saveGatewayToken(db, key, agentId, "telegram", "bot-token-value", "my_bot");
    await db.delete(hvAgents).where(eq(hvAgents.id, agentId)).run();
    expect(await listGateways(db, "telegram")).toEqual([]);
    expect(gatewayStatus(undefined)).toEqual({ hasToken: false, connected: false, botUsername: null, chatId: null, chatUsername: null });
  });

  it("stores one gateway row per agent and type", async () => {
    const agentId = await seedAgent();
    await saveGatewayToken(db, key, agentId, "telegram", "token-one", "my_bot");
    // Re-adding for the same agent must not silently duplicate; the store only inserts.
    await expect(saveGatewayToken(db, key, agentId, "telegram", "token-two", "my_bot")).rejects.toThrow();
    const rows = await db.select().from(hvGateways).all();
    expect(rows).toHaveLength(1);
    expect((rows[0] as GatewayRow).botUsername).toBe("my_bot");
  });
});
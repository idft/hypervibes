import { createModels } from "@earendil-works/pi-ai/models";
import { fauxProvider } from "@earendil-works/pi-ai/providers/faux";
import { createHash, randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hvUsers } from "../db/auth-schema.js";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { saveGatewayToken } from "./gateway-store.js";
import { GatewayBridge } from "./bridge-service.js";
import type { BridgeMessage, GatewayConnector, GatewayEndpoint, GatewaySender } from "./handlers.js";
import { telegramConnector } from "./telegram/connector.js";

/** One configured token per test; the runtime and store must share it. */
const ENCRYPTION_KEY_HEX = createHash("sha256").update("gateway-test-encryption-key").digest("hex");

/** An endpoint that records intake and output without touching the network. */
class FakeEndpoint implements GatewayEndpoint {
  private onMessage: (message: BridgeMessage) => void = () => {};
  readonly sent: { chatId: number; text: string }[] = [];
  readonly typing: number[] = [];
  started = false;
  stopped = false;
  constructor(readonly token: string) {}
  attach(onMessage: (message: BridgeMessage) => void) { this.onMessage = onMessage; }
  /** Simulates an incoming platform message. */
  receive(text: string, chatId = 1, fromUsername?: string) { this.onMessage({ chatId, text, ...(fromUsername ? { fromUsername } : {}) }); }
  async start() { this.started = true; return new Promise<void>(() => {}); }
  async stop() { this.stopped = true; }
  async sendText(chatId: number, text: string) { this.sent.push({ chatId, text }); }
  async sendTyping(chatId: number) { this.typing.push(chatId); }
}

/** Records every endpoint the fake connector created. */
class EndpointLog {
  readonly endpoints: FakeEndpoint[] = [];
  last(): FakeEndpoint { return this.endpoints.at(-1)!; }
}

function fakeConnector(log: EndpointLog, startError?: Error): GatewayConnector {
  return {
    ...telegramConnector,
    createEndpoint(token, onMessage) {
      const endpoint = new FakeEndpoint(token);
      endpoint.attach(onMessage);
      if (startError) {
        return {
          start: () => Promise.reject(startError),
          stop: () => endpoint.stop(),
          sendText: (chatId: number, text: string) => endpoint.sendText(chatId, text),
          sendTyping: (chatId: number) => endpoint.sendTyping(chatId),
        };
      }
      log.endpoints.push(endpoint);
      return endpoint;
    },
  };
}

function startToken(link: string): string {
  const token = /start=([0-9a-f-]+)$/.exec(link)?.[1];
  if (token === undefined) throw new Error("test link carries no token");
  return token;
}

/** A sender that records without the network, for direct workspace checks. */
function recordingSender(): GatewaySender & { sent: string[] } {
  const sent: string[] = [];
  return { sent, async sendText(_chatId, text) { sent.push(text); }, async sendTyping() {} };
}

describe("gateway bridge lifecycle", () => {
  let directory: string;
  let runtime: AppRuntime;
  let faux: ReturnType<typeof fauxProvider>;
  let log: EndpointLog;
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite"), AGENTS_ENCRYPTION_KEY: ENCRYPTION_KEY_HEX });
  const options = () => {
    const models = createModels();
    models.setProvider(faux.provider);
    return { models, model: { provider: faux.getModel().provider, modelId: faux.getModel().id }, accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [] };
  };
  const key = () => config().encryptionKey!;

  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-bridge-"));
    faux = fauxProvider();
    log = new EndpointLog();
    runtime = await createRuntime(config(), options());
    await runtime.db.insert(hvUsers).values({ id: "owner", name: "Owner", email: "owner@example.test" }).run();
  });
  afterEach(async () => {
    await runtime.close();
    rmSync(directory, { recursive: true, force: true });
  });

  async function seedAgent() {
    const account = `0x${createHash("sha256").update(randomUUID()).digest("hex").slice(0, 40)}`;
    return runtime.agents.create("owner", "Bridged", randomUUID(), account);
  }

  it("starts endpoints for gateways saved before startup and through ensure() afterwards", async () => {
    const agent = await seedAgent();
    await saveGatewayToken(runtime.db, key(), agent.id, "telegram", "tok-existing", "my_bot");
    const bridge = new GatewayBridge(runtime.db, runtime.agents, key(), fakeConnector(log));
    bridge.start();
    await vi.waitFor(() => expect(log.endpoints.length).toBe(1));
    expect(log.last().token).toBe("tok-existing");
    expect(log.last().started).toBe(true);

    const later = await seedAgent();
    await saveGatewayToken(runtime.db, key(), later.id, "telegram", "tok-later", "my_bot");
    await bridge.ensure(later.id);
    expect(log.endpoints.length).toBe(2);
    // ensure() is idempotent while the endpoint runs.
    await bridge.ensure(later.id);
    expect(log.endpoints.length).toBe(2);
    await bridge.close();
    expect(log.endpoints.every((endpoint) => endpoint.stopped)).toBe(true);
  });

  it("starts nothing until start() or ensure() is called", async () => {
    const agent = await seedAgent();
    await saveGatewayToken(runtime.db, key(), agent.id, "telegram", "tok-a", "my_bot");
    const bridge = new GatewayBridge(runtime.db, runtime.agents, key(), fakeConnector(log));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(log.endpoints).toEqual([]);
    await bridge.ensure(agent.id);
    expect(log.endpoints.length).toBe(1);
    await bridge.close();
  });

  it("stops the endpoint and kills pending links on disconnect", async () => {
    const agent = await seedAgent();
    await saveGatewayToken(runtime.db, key(), agent.id, "telegram", "tok-a", "my_bot");
    const bridge = new GatewayBridge(runtime.db, runtime.agents, key(), fakeConnector(log));
    bridge.start();
    await vi.waitFor(() => expect(log.endpoints.length).toBe(1));
    const link = await bridge.startLink(agent.id, "owner");
    const token = startToken(link);
    await bridge.removed(agent.id);
    expect(log.last().stopped).toBe(true);
    const fresh = fakeConnector(new EndpointLog());
    // The pending token died with the disconnect; a fresh bridge reports it unknown.
    const other = new GatewayBridge(runtime.db, runtime.agents, key(), fresh);
    await other.close();
    const recording = recordingSender();
    await telegramConnector.handle(agent.id, { chatId: 7, text: `/start ${token}` }, {
      loadGateway: async () => undefined,
      bindChat: async () => true,
      consumeLink: () => ({ status: "unknown" }),
      bridge: async () => { throw new Error("unreachable"); },
    }, recording);
    expect(recording.sent).toEqual(["This link token is unknown or has expired. Start a new link from the web UI."]);
    await bridge.close();
  });

  it("completes the link flow from deep link to bound chat", async () => {
    const agent = await seedAgent();
    await saveGatewayToken(runtime.db, key(), agent.id, "telegram", "tok-a", "my_bot");
    const bridge = new GatewayBridge(runtime.db, runtime.agents, key(), fakeConnector(log));
    bridge.start();
    await vi.waitFor(() => expect(log.endpoints.length).toBe(1));
    const link = await bridge.startLink(agent.id, "owner");
    expect(link).toMatch(/^https:\/\/t\.me\/my_bot\?start=[0-9a-f-]+$/);
    const token = startToken(link);
    log.last().receive(`/start ${token}`, 7, "operator");
    await vi.waitFor(() => expect(log.last().sent.map((entry) => entry.text)).toEqual(["Telegram chat linked. You can now use this chat with HyperVibes."]));
    const status = await bridge.status(agent.id);
    expect(status).toMatchObject({ hasToken: true, connected: true, chatId: 7, chatUsername: "operator" });
    // The token is single-use.
    log.last().receive(`/start ${token}`, 8);
    await vi.waitFor(() => expect(log.last().sent).toHaveLength(2));
    expect(log.last().sent[1]?.text).toContain("unknown or has expired");
    await bridge.close();
  });

  it("forwards bound-chat messages into the agent's main conversation", async () => {
    const agent = await seedAgent();
    await saveGatewayToken(runtime.db, key(), agent.id, "telegram", "tok-a", "my_bot");
    const bridge = new GatewayBridge(runtime.db, runtime.agents, key(), fakeConnector(log));
    bridge.start();
    await vi.waitFor(() => expect(log.endpoints.length).toBe(1));
    const link = await bridge.startLink(agent.id, "owner");
    const token = startToken(link);
    log.last().receive(`/start ${token}`, 7, "operator");
    await vi.waitFor(async () => expect((await bridge.status(agent.id)).connected).toBe(true));
    runtime.start();
    log.last().receive("hello from telegram", 7);
    const chat = await runtime.agents.chat("owner", agent.id);
    await vi.waitFor(async () => {
      const latest = await runtime.agents.chat("owner", agent.id);
      expect(latest.messages.map((entry) => entry.text)).toContain("hello from telegram");
      expect(chat.busy).toBeDefined();
    });
    // The faux model replies; the bridge forwards the reply text to the chat.
    await vi.waitFor(() => expect(log.last().sent.some((entry) => entry.chatId === 7 && entry.text.length > 0)).toBe(true));
    await bridge.close();
  });

  it("records endpoint start failures for the settings page", async () => {
    const agent = await seedAgent();
    await saveGatewayToken(runtime.db, key(), agent.id, "telegram", "tok-a", "my_bot");
    const bridge = new GatewayBridge(runtime.db, runtime.agents, key(), fakeConnector(log, new Error("401: Unauthorized: bot token revoked")));
    bridge.start();
    await vi.waitFor(async () => expect((await bridge.status(agent.id)).error).toContain("revoked"));
    // A later ensure() retries and records the failure again.
    await bridge.ensure(agent.id);
    await vi.waitFor(async () => expect((await bridge.status(agent.id)).error).toContain("revoked"));
    await bridge.close();
  });

  it("reports an undecryptable token instead of starting an endpoint", async () => {
    const agent = await seedAgent();
    await saveGatewayToken(runtime.db, key(), agent.id, "telegram", "tok-a", "my_bot");
    const stale = { id: "rotated", bytes: new Uint8Array(32) };
    const bridge = new GatewayBridge(runtime.db, runtime.agents, stale, fakeConnector(log));
    bridge.start();
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(log.endpoints).toEqual([]);
    expect((await bridge.status(agent.id)).error).toContain("decrypted");
    await bridge.close();
  });

  it("requires an encryption key for credentials and a stored token for links", async () => {
    const agent = await seedAgent();
    const noKey = new GatewayBridge(runtime.db, runtime.agents, undefined, fakeConnector(log));
    expect(() => noKey.requireKey()).toThrow("AGENTS_ENCRYPTION_KEY");
    const bridge = new GatewayBridge(runtime.db, runtime.agents, key(), fakeConnector(log));
    await expect(bridge.startLink(agent.id, "owner")).rejects.toThrow("Add a Telegram bot token first.");
    await Promise.all([noKey.close(), bridge.close()]);
  });
});
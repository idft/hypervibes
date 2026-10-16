import { envApiKeyAuth, isModelType, type OAuthCredential } from "@earendil-works/pi-ai";
import { fauxAssistantMessage, fauxProvider } from "@earendil-works/pi-ai/providers/faux";
import type { Provider } from "@earendil-works/pi-ai/models";
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hvUsers } from "../db/auth-schema.js";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { SqliteCredentials } from "./credential-store.js";
import type { LoginView } from "./provider-service.js";

const secretKey = "19".repeat(32);
const ref = { provider: "test-provider", modelId: "small" };

describe("per-user providers", () => {
  let directory: string;
  let runtime: AppRuntime;
  let faux: ReturnType<typeof fauxProvider>;
  let refresh: ReturnType<typeof vi.fn<(credential: OAuthCredential) => Promise<OAuthCredential>>>;
  let seenAuth: (string | undefined)[];
  let seenModels: (typeof ref)[];
  const config = (key = secretKey) => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite"), AGENTS_ENCRYPTION_KEY: key });
  const provider = (): Provider => ({
    ...faux.provider,
    auth: {
      apiKey: envApiKeyAuth("Test API key", ["HV_TEST_PROVIDER_API_KEY"]),
      oauth: {
        name: "Test subscription", isSubscription: true,
        login: async (interaction, options) => {
          expect(options?.getDeviceId?.()).toMatch(/^[0-9a-f-]{36}$/);
          interaction.notify({ type: "auth_url", url: "https://auth.example.test/authorize", instructions: "Paste the callback URL." });
          const code = await interaction.prompt({ type: "manual_code", message: "Callback URL", signal: interaction.signal });
          if (code !== "valid-callback") throw new Error("secret upstream response body");
          return { type: "oauth", access: "access-secret", refresh: "refresh-secret", expires: Date.now() + 3_600_000 };
        },
        refresh: (credential) => refresh(credential),
        toAuth: async (credential) => ({ apiKey: credential.access }),
      },
    },
    streamSimple: (model, context, options) => {
      seenAuth.push(options?.apiKey);
      seenModels.push({ provider: model.provider, modelId: model.id });
      return faux.provider.streamSimple(model, context, options);
    },
  });
  const options = () => ({ providers: () => [provider()], accountFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [] });
  async function selectModel(userId: string, agentId: string, model: typeof ref, thinkingLevel = "") {
    const { definition } = await runtime.agents.subagents.detail(userId, agentId, `main:${agentId}`);
    return runtime.agents.subagents.update(userId, agentId, definition.id, { ...definition, ...model, thinkingLevel });
  }
  async function prompt(id: string, user = "owner") {
    let result!: LoginView;
    await vi.waitFor(() => {
      result = runtime.providers.status(user, id);
      expect(result.prompt).toBeDefined();
    });
    return result.prompt!;
  }
  async function connect(user = "owner", value = "key-secret", type: "api_key" | "oauth" = "api_key") {
    const started = await runtime.providers.begin(user, ref.provider, type);
    const input = await prompt(started.id, user);
    runtime.providers.answer(user, started.id, input.id, value);
    await vi.waitFor(() => expect(runtime.providers.status(user, started.id).state).toBe("complete"));
    return started.id;
  }
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-providers-"));
    seenAuth = [];
    seenModels = [];
    faux = fauxProvider({ provider: ref.provider, models: [{ id: "small" }, { id: "large", reasoning: true }] });
    refresh = vi.fn(async (credential: OAuthCredential) => ({ ...credential, access: "renewed-access", refresh: "rotated-refresh", expires: Date.now() + 3_600_000 }));
    runtime = await createRuntime(config(), options());
    await runtime.db.insert(hvUsers).values([
      { id: "owner", name: "Owner", email: "owner@example.test" },
      { id: "other", name: "Other", email: "other@example.test" },
    ]).run();
    runtime.start();
  });
  afterEach(async () => {
    vi.unstubAllEnvs();
    await runtime.close();
    rmSync(directory, { recursive: true, force: true });
  });

  it("registers all built-ins including Codex OAuth, with key-only cloud account setup", async () => {
    await runtime.close();
    runtime = await createRuntime(config());
    const settings = await runtime.providers.settings("owner");
    expect(settings.providers.map((provider) => provider.id)).toEqual(expect.arrayContaining(["openai", "openai-codex", "anthropic", "opencode-go", "openrouter"]));
    expect(settings.providers.find((provider) => provider.id === "openai-codex")?.methods.some((method) => method.type === "oauth")).toBe(true);
    expect(settings.models).toEqual([]);
    for (const providerId of ["amazon-bedrock", "google-vertex"]) {
      const attempt = await runtime.providers.begin("owner", providerId, "api_key");
      expect((await prompt(attempt.id)).input.type).toBe("secret");
      await runtime.providers.cancel("owner", attempt.id);
    }
  });

  it("does not inherit server env credentials; requires an available selected model for creation", async () => {
    vi.stubEnv("HV_TEST_PROVIDER_API_KEY", "server-key-secret");
    expect(await runtime.providers.choices("owner")).toEqual([]);
    expect(await (await runtime.providers.models("owner")).getAuth(ref.provider)).toBeUndefined();
    await expect(runtime.agents.create("owner", "Blocked", randomUUID(), `0x${"12".repeat(20)}`, ref)).rejects.toMatchObject({ status: 409 });
    expect(await runtime.agents.list("owner")).toEqual([]);
    expect(faux.state.callCount).toBe(0);
  });

  it("encrypts credentials, exposes only metadata, isolates user collections and persists across restart", async () => {
    await connect();
    await connect("other", "other-secret");
    expect((await (await runtime.providers.models("owner")).getAuth(ref.provider))?.auth.apiKey).toBe("key-secret");
    expect((await (await runtime.providers.models("other")).getAuth(ref.provider))?.auth.apiKey).toBe("other-secret");
    expect(await runtime.providers.models("owner")).not.toBe(await runtime.providers.models("other"));
    const settings = await runtime.providers.settings("owner");
    expect(settings.models).toHaveLength(2);
    expect(settings.providers[0]).toMatchObject({ connected: true, configured: true, authType: "api_key" });
    expect(JSON.stringify(settings)).not.toContain("key-secret");
    const rows = await runtime.database.all<{ ciphertext: Uint8Array }>("SELECT ciphertext FROM hv_provider_connections");
    expect(rows).toHaveLength(2);
    expect(rows.every((row) => !Buffer.from(row.ciphertext).toString().includes("secret"))).toBe(true);
    const installation = await runtime.database.get("SELECT device_id FROM hv_installation");
    await runtime.close();
    runtime = await createRuntime(config(), options());
    expect((await runtime.providers.settings("owner")).models).toHaveLength(2);
    expect(await runtime.database.get("SELECT device_id FROM hv_installation")).toEqual(installation);
    expect((await (await runtime.providers.models("other")).getAuth(ref.provider))?.auth.apiKey).toBe("other-secret");
  });

  it("bridges OAuth events and manual prompts, rejects foreign attempts and stale prompt answers", async () => {
    const attempt = await runtime.providers.begin("owner", ref.provider, "oauth");
    const input = await prompt(attempt.id);
    expect(runtime.providers.status("owner", attempt.id).events[0]).toMatchObject({ type: "auth_url" });
    expect(input.input.type).toBe("manual_code");
    expect(() => runtime.providers.status("other", attempt.id)).toThrow("not found");
    expect(() => runtime.providers.answer("other", attempt.id, input.id, "valid-callback")).toThrow("not found");
    await expect(runtime.providers.cancel("other", attempt.id)).rejects.toMatchObject({ status: 404 });
    expect(() => runtime.providers.answer("owner", attempt.id, "stale", "valid-callback")).toThrow("expired");
    runtime.providers.answer("owner", attempt.id, input.id, "valid-callback");
    await vi.waitFor(() => expect(runtime.providers.status("owner", attempt.id).state).toBe("complete"));
    expect((await runtime.providers.settings("owner")).providers[0]?.authType).toBe("oauth");
    expect(JSON.stringify(await runtime.providers.settings("owner"))).not.toContain("refresh-secret");
  });

  it("supports optional text and multiple prompts, validates selections and blocks arbitrary Copilot endpoints", async () => {
    await runtime.close();
    runtime = await createRuntime(config(), { providers: () => [{
      ...fauxProvider({ provider: "github-copilot" }).provider,
      auth: { apiKey: { ...envApiKeyAuth("Test", []), login: async (interaction) => {
        expect(await interaction.prompt({ type: "text", message: "Domain (blank for github.com)" })).toBe("");
        expect(await interaction.prompt({ type: "select", message: "Method", options: [{ id: "key", label: "API key" }] })).toBe("key");
        return { type: "api_key", key: await interaction.prompt({ type: "secret", message: "Key" }) };
      } } },
    }] });
    const attempt = await runtime.providers.begin("owner", "github-copilot", "api_key");
    const domain = await prompt(attempt.id);
    expect(() => runtime.providers.answer("owner", attempt.id, domain.id, "https://localhost")).toThrow("Custom enterprise");
    runtime.providers.answer("owner", attempt.id, domain.id, "");
    const method = await prompt(attempt.id);
    expect(() => runtime.providers.answer("owner", attempt.id, method.id, "not-an-option")).toThrow("Select one");
    runtime.providers.answer("owner", attempt.id, method.id, "key");
    const key = await prompt(attempt.id);
    expect(() => runtime.providers.answer("owner", attempt.id, key.id, "")).toThrow("valid sign-in");
    runtime.providers.answer("owner", attempt.id, key.id, "secret");
    await vi.waitFor(() => expect(runtime.providers.status("owner", attempt.id).state).toBe("complete"));
  });

  it("cancels and times out sign-ins without storing credentials or leaking upstream errors", async () => {
    const attempt = await runtime.providers.begin("owner", ref.provider, "oauth");
    await prompt(attempt.id);
    await expect(runtime.providers.begin("other", ref.provider, "oauth")).rejects.toMatchObject({ status: 409 });
    await runtime.providers.cancel("owner", attempt.id);
    expect(runtime.providers.status("owner", attempt.id).state).toBe("cancelled");
    expect(await runtime.providers.choices("owner")).toEqual([]);
    const failure = await runtime.providers.begin("owner", ref.provider, "oauth");
    const input = await prompt(failure.id);
    runtime.providers.answer("owner", failure.id, input.id, "invalid");
    await vi.waitFor(() => expect(runtime.providers.status("owner", failure.id).state).toBe("failed"));
    expect(JSON.stringify(runtime.providers.status("owner", failure.id))).not.toContain("secret upstream");
    vi.useFakeTimers();
    try {
      const expired = await runtime.providers.begin("owner", ref.provider, "api_key");
      await vi.advanceTimersByTimeAsync(10 * 60_000);
      expect(() => runtime.providers.status("owner", expired.id)).toThrow("expired");
    } finally { vi.useRealTimers(); }
  });

  it("serializes OAuth refresh without holding the shared SQL queue, persists rotated tokens and prevents resurrection on logout", async () => {
    await connect("owner", "valid-callback", "oauth");
    const store = new SqliteCredentials(runtime.db, "owner", config().encryptionKey);
    await store.modify(ref.provider, async (current) => ({ ...current as OAuthCredential, expires: 0 }));
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    refresh.mockImplementation(async (credential) => {
      await gate;
      return { ...credential, access: "renewed-access", refresh: "rotated-refresh", expires: Date.now() + 3_600_000 };
    });
    const models = await runtime.providers.models("owner");
    const first = models.getAuth(ref.provider);
    const second = models.getAuth(ref.provider);
    await vi.waitFor(() => expect(refresh).toHaveBeenCalledOnce());
    expect(await runtime.database.get("SELECT 1 AS available")).toEqual({ available: 1 });
    const logout = runtime.providers.disconnect("owner", ref.provider);
    release();
    await Promise.all([first, second, logout]);
    expect(refresh).toHaveBeenCalledOnce();
    expect(await store.read(ref.provider)).toBeUndefined();
    expect(await models.getAuth(ref.provider)).toBeUndefined();
  });

  it("preserves credentials on refresh failure and never falls back to a server key", async () => {
    await connect("owner", "valid-callback", "oauth");
    const store = new SqliteCredentials(runtime.db, "owner", config().encryptionKey);
    await store.modify(ref.provider, async (current) => ({ ...current as OAuthCredential, expires: 0 }));
    refresh.mockRejectedValue(new Error("secret provider failure"));
    vi.stubEnv("HV_TEST_PROVIDER_API_KEY", "server-key");
    await expect((await runtime.providers.models("owner")).getAuth(ref.provider)).rejects.toMatchObject({ code: "oauth" });
    expect((await store.read(ref.provider))?.type).toBe("oauth");
    expect(JSON.stringify(await runtime.providers.settings("owner"))).not.toContain("secret provider failure");
  });

  it("creates and changes a model in the same conversation, persists selection, and blocks disconnected inference", async () => {
    await connect();
    await expect(runtime.agents.create("owner", "Invalid", randomUUID(), `0x${"12".repeat(20)}`, { ...ref, modelId: "missing" })).rejects.toMatchObject({ status: 409 });
    const requestId = randomUUID();
    const agent = await runtime.agents.create("owner", "Research", requestId, `0x${"12".repeat(20)}`, ref);
    await expect(runtime.agents.create("owner", "Research", requestId, `0x${"12".repeat(20)}`, { ...ref, modelId: "large" })).rejects.toMatchObject({ status: 409 });
    expect(faux.state.callCount).toBe(0);
    faux.setResponses([fauxAssistantMessage("First answer"), fauxAssistantMessage("Second answer")]);
    await runtime.agents.send("owner", agent.id, "Hello", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages.map((entry) => entry.text)).toEqual(["Hello", "First answer"]));
    await expect(selectModel("other", agent.id, ref)).rejects.toMatchObject({ status: 404 });
    await expect(selectModel("owner", agent.id, { ...ref, modelId: "large" }, "nonsense")).rejects.toThrow("thinking");
    await selectModel("owner", agent.id, { ...ref, modelId: "large" });
    expect((await runtime.agents.chat("owner", agent.id)).model.modelId).toBe("large");
    expect((await runtime.agents.chat("owner", agent.id)).agent.conversationId).toBe(agent.conversationId);
    await runtime.agents.send("owner", agent.id, "Again", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages).toHaveLength(4));
    expect(seenAuth).toEqual(["key-secret", "key-secret"]);
    expect(seenModels).toEqual([ref, { ...ref, modelId: "large" }]);
    await runtime.close();
    runtime = await createRuntime(config(), options());
    runtime.start();
    expect((await runtime.agents.chat("owner", agent.id)).model.modelId).toBe("large");
    expect((await runtime.agents.chat("owner", agent.id)).messages).toHaveLength(4);
    faux.setResponses([fauxAssistantMessage("After restart")]);
    await runtime.agents.send("owner", agent.id, "After restart", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages).toHaveLength(6));
    expect(seenModels).toEqual([ref, { ...ref, modelId: "large" }, { ...ref, modelId: "large" }]);
    await runtime.providers.disconnect("owner", ref.provider);
    const disconnected = await runtime.agents.chat("owner", agent.id);
    expect(disconnected.configured).toBe(false);
    expect(disconnected.messages).toHaveLength(6);
    await expect(runtime.agents.send("owner", agent.id, "Blocked", randomUUID())).rejects.toMatchObject({ status: 409 });
    await expect(runtime.agents.create("owner", "Blocked", randomUUID(), `0x${"12".repeat(20)}`, ref)).rejects.toMatchObject({ status: 409 });
    await connect("owner", "replacement-key");
    expect((await runtime.agents.chat("owner", agent.id)).configured).toBe(true);
  });

  it("uses the selected model and the user's OAuth credential for inference", async () => {
    await connect("owner", "valid-callback", "oauth");
    const selected = { ...ref, modelId: "large" };
    const agent = await runtime.agents.create("owner", "OAuth agent", randomUUID(), `0x${"12".repeat(20)}`, selected);
    faux.setResponses([fauxAssistantMessage("OAuth answer")]);
    await runtime.agents.send("owner", agent.id, "Hello", randomUUID());
    await vi.waitFor(async () => expect((await runtime.agents.chat("owner", agent.id)).messages).toHaveLength(2));
    expect(seenModels).toEqual([selected]);
    expect(seenAuth).toEqual(["access-secret"]);
  });

  it("pushes provider availability and model changes to existing chat subscriptions", async () => {
    await connect();
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`, ref);
    const controller = new AbortController();
    const updates: { configured: boolean; modelId: string }[] = [];
    const watching = runtime.agents.watchChat("owner", agent.id, controller.signal, (chat) => {
      updates.push({ configured: chat.configured, modelId: chat.model.modelId });
    });
    try {
      await vi.waitFor(() => expect(updates.at(-1)?.configured).toBe(true));
      await runtime.providers.disconnect("owner", ref.provider);
      await vi.waitFor(() => expect(updates.at(-1)?.configured).toBe(false));
      await connect();
      await vi.waitFor(() => expect(updates.at(-1)?.configured).toBe(true));
      await selectModel("owner", agent.id, { ...ref, modelId: "large" });
      await vi.waitFor(() => expect(updates.at(-1)?.modelId).toBe("large"));
    } finally { controller.abort(); await watching; }
  });

  it("refuses model changes while work is active and does not change another user's credentials on disconnect", async () => {
    await connect();
    await connect("other", "other-key");
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`, ref);
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    faux.setResponses([async () => { await gate; return fauxAssistantMessage("Done"); }]);
    await runtime.agents.send("owner", agent.id, "Start", randomUUID());
    await vi.waitFor(() => expect(faux.state.callCount).toBe(1));
    try {
      await expect(selectModel("owner", agent.id, { ...ref, modelId: "large" })).rejects.toMatchObject({ status: 409 });
      await runtime.providers.disconnect("other", ref.provider);
      expect((await runtime.agents.chat("owner", agent.id)).configured).toBe(true);
    } finally { release(); }
  });

  it("fails closed with a missing/wrong encryption key and still permits existing history", async () => {
    await connect();
    const agent = await runtime.agents.create("owner", "Research", randomUUID(), `0x${"12".repeat(20)}`, ref);
    await runtime.close();
    runtime = await createRuntime(config(""), options());
    expect((await runtime.agents.chat("owner", agent.id)).configured).toBe(false);
    expect((await runtime.providers.settings("owner")).storageReady).toBe(false);
    await expect(runtime.providers.begin("owner", ref.provider, "api_key")).rejects.toMatchObject({ status: 503 });
    await runtime.close();
    runtime = await createRuntime(config("42".repeat(32)), options());
    expect((await runtime.providers.settings("owner")).providers[0]).toMatchObject({ connected: true, configured: false });
    await expect((await runtime.providers.models("owner")).getAuth(ref.provider)).rejects.toMatchObject({ code: "auth" });
  });

  it("restores per-user dynamic catalogs offline on restart", async () => {
    await runtime.close();
    let networkRefreshes = 0;
    const dynamic = () => {
      let catalog: ReturnType<Provider["getModels"]> = [];
      return [{
        ...provider(), getModels: () => catalog,
        refreshModels: async (context) => {
          if (context.stored) {
            const restored = context.stored.models.filter((model) => isModelType(model, "chat"));
            await context.publish({ update: () => { catalog = restored; } });
          }
          if (context.allowNetwork) {
            networkRefreshes++;
            await context.publish({ persist: { models: faux.models }, update: () => { catalog = faux.models; } });
          }
        },
      } satisfies Provider];
    };
    runtime = await createRuntime(config(), { providers: dynamic });
    await connect();
    expect(networkRefreshes).toBe(1);
    expect(await runtime.providers.choices("owner")).toHaveLength(2);
    const agent = await runtime.agents.create("owner", "Dynamic", randomUUID(), `0x${"12".repeat(20)}`, ref);
    await runtime.close();
    runtime = await createRuntime(config(), { providers: dynamic });
    expect(networkRefreshes).toBe(1);
    expect(await runtime.providers.choices("owner")).toHaveLength(2);
    expect(await runtime.providers.choices("other")).toEqual([]);
    expect((await runtime.agents.chat("owner", agent.id)).configured).toBe(true);
  });

  it("rejects ciphertext copied between users or tampered with", async () => {
    await connect();
    await connect("other", "other-key");
    await runtime.database.run("UPDATE hv_provider_connections SET ciphertext = (SELECT ciphertext FROM hv_provider_connections WHERE user_id = 'owner') WHERE user_id = 'other'");
    expect(await runtime.providers.choices("other")).toEqual([]);
    expect(await runtime.providers.choices("owner")).toHaveLength(2);
    await runtime.database.run("UPDATE hv_provider_connections SET ciphertext = zeroblob(32) WHERE user_id = 'owner'");
    expect(await runtime.providers.choices("owner")).toEqual([]);
  });

  it("reserves one login under concurrent requests, including after a prior completion", async () => {
    await connect();
    const attempts = await Promise.allSettled([
      runtime.providers.begin("owner", ref.provider, "api_key"), runtime.providers.begin("owner", ref.provider, "api_key"),
    ]);
    expect(attempts.filter((attempt) => attempt.status === "fulfilled")).toHaveLength(1);
    expect(attempts.filter((attempt) => attempt.status === "rejected")).toHaveLength(1);
  });

  it("cancels outstanding login prompts on shutdown and leaves no credentials", async () => {
    const attempt = await runtime.providers.begin("owner", ref.provider, "oauth");
    await prompt(attempt.id);
    await runtime.close();
    runtime = await createRuntime(config(), options());
    expect(await runtime.providers.choices("owner")).toEqual([]);
    expect((await runtime.providers.settings("owner")).activeLogin).toBeUndefined();
  });
});

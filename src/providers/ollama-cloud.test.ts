import { type ModelsPublication, type RefreshModelsContext } from "@earendil-works/pi-ai/models";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hvUsers } from "../db/auth-schema.js";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { fetchModels, ollamaCloudProvider } from "./ollama-cloud.js";
import type { LoginView } from "./provider-service.js";

const secretKey = "19".repeat(32);

/** Fixture catalog: a tools model, a vision model, and one without tool support. */
const catalog: Record<string, { capabilities: string[]; model_info?: Record<string, unknown> }> = {
  "qwen3-coder:cloud": { capabilities: ["tools"], model_info: { "general.context_length": 262_144 } },
  "llava:cloud": { capabilities: ["tools", "vision"] },
  "embedding:cloud": { capabilities: ["completion"] },
};

const state = { failing: new Set<string>(), listed: undefined as string[] | undefined };
const seen: { url: string; authorization?: string }[] = [];

function respond(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

const fetchStub = vi.fn(async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
  const url = String(input);
  const authorization = new Headers(init?.headers).get("Authorization") ?? undefined;
  seen.push({ url, authorization });
  if (url === "https://ollama.com/v1/models") {
    return respond({ data: (state.listed ?? Object.keys(catalog)).map((id) => ({ id })) });
  }
  if (url === "https://ollama.com/api/show") {
    const { model } = JSON.parse(String(init?.body)) as { model: string };
    if (state.failing.has(model)) return respond({ error: "overloaded" }, 503);
    const details = catalog[model];
    if (!details) return respond({ error: "not found" }, 404);
    return respond({ capabilities: details.capabilities, model_info: details.model_info ?? {} });
  }
  throw new Error(`unexpected fetch: ${url}`);
});

function refreshContext(): { context: RefreshModelsContext; published: ModelsPublication[] } {
  const published: ModelsPublication[] = [];
  const context: RefreshModelsContext = {
    credential: { type: "api_key", key: "key-secret" },
    allowNetwork: true,
    signal: new AbortController().signal,
    publish: async (publication) => {
      published.push(publication);
      publication.update?.();
      return true;
    },
  };
  return { context, published };
}

describe("ollama cloud provider", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", fetchStub);
    fetchStub.mockClear();
    state.failing.clear();
    state.listed = undefined;
    seen.length = 0;
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("streams through the OpenAI completions API and exposes only api-key login", () => {
    const provider = ollamaCloudProvider();
    expect(provider.id).toBe("ollama-cloud");
    expect(provider.getModels()).toEqual([]);
    expect(provider.auth.apiKey?.login).toBeDefined();
    expect(provider.auth.oauth).toBeUndefined();
  });

  it("fetches the live catalog with the user's key and publishes tools-capable models", async () => {
    const { context, published } = refreshContext();
    const provider = ollamaCloudProvider();
    await provider.refreshModels!(context);
    expect(published).toHaveLength(1);
    expect(published[0]!.persist?.models).toHaveLength(2);
    expect(published[0]!.persist?.checkedAt).toBeTypeOf("number");
    const models = provider.getModels();
    expect(models.map((model) => model.id)).toEqual(["qwen3-coder:cloud", "llava:cloud"]);
    expect(models.every((model) => model.provider === "ollama-cloud" && model.api === "openai-completions" && model.baseUrl === "https://ollama.com/v1")).toBe(true);
    const coder = models.find((model) => model.id === "qwen3-coder:cloud")!;
    expect(coder.contextWindow).toBe(262_144);
    expect(coder.input).toEqual(["text"]);
    expect(coder.reasoning).toBe(false);
    expect(coder.compat).toMatchObject({ supportsDeveloperRole: false, supportsStore: false, maxTokensField: "max_tokens" });
    expect(models.find((model) => model.id === "llava:cloud")!.input).toEqual(["text", "image"]);
    expect(seen.every((request) => request.authorization === "Bearer key-secret")).toBe(true);
  });

  it("restores a stored catalog without network access", async () => {
    const provider = ollamaCloudProvider();
    const first = refreshContext();
    await provider.refreshModels!(first.context);
    const stored = first.published[0]!.persist!;
    fetchStub.mockClear();
    const restore = refreshContext();
    restore.context.credential = undefined;
    restore.context.stored = stored;
    restore.context.allowNetwork = false;
    await provider.refreshModels!(restore.context);
    expect(fetchStub).not.toHaveBeenCalled();
    expect(restore.published[0]!.persist).toBeUndefined();
    expect(provider.getModels().map((model) => model.id)).toEqual(["qwen3-coder:cloud", "llava:cloud"]);
  });

  it("throws and publishes nothing when the catalog is incomplete or unusable", async () => {
    state.failing.add("llava:cloud");
    const { context, published } = refreshContext();
    await expect(fetchModels(context)).rejects.toThrow(/1 of 3 model\(s\) failed/);
    expect(published).toHaveLength(0);

    state.failing.clear();
    state.listed = ["embedding:cloud"];
    await expect(fetchModels(refreshContext().context)).rejects.toThrow("no models with tool support");
  });

  it("refreshes live per user, isolates collections, and restores offline on restart", async () => {
    const directory = mkdtempSync(join(tmpdir(), "hv-ollama-"));
    const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "app.sqlite"), AGENTS_ENCRYPTION_KEY: secretKey });
    const options = () => ({
      providers: () => [ollamaCloudProvider()],
      accountFeedFactory: () => ({ close: async () => {} }),
      journalHistoryFetcher: async () => [],
    });
    let runtime: AppRuntime;
    const connect = async (user: string, key: string) => {
      const started = await runtime.providers.begin(user, "ollama-cloud", "api_key");
      let prompt: LoginView["prompt"];
      await vi.waitFor(() => {
        prompt = runtime.providers.status(user, started.id).prompt;
        expect(prompt).toBeDefined();
      });
      runtime.providers.answer(user, started.id, prompt!.id, key);
      await vi.waitFor(() => expect(runtime.providers.status(user, started.id).state).toBe("complete"));
    };
    runtime = await createRuntime(config(), options());
    try {
      await runtime.db.insert(hvUsers).values([
        { id: "owner", name: "Owner", email: "owner@example.test" },
        { id: "other", name: "Other", email: "other@example.test" },
      ]).run();
      runtime.start();
      await connect("owner", "key-secret");
      const choices = await runtime.providers.choices("owner");
      expect(choices.map((choice) => choice.modelId)).toEqual(["qwen3-coder:cloud", "llava:cloud"]);
      expect(choices.every((choice) => choice.providerName === "Ollama Cloud" && choice.contextWindow > 0)).toBe(true);
      expect(seen.every((request) => request.authorization === "Bearer key-secret")).toBe(true);
      expect(await runtime.providers.choices("other")).toEqual([]);

      // Restart restores the per-user catalog without network access.
      const networkCalls = fetchStub.mock.calls.length;
      await runtime.close();
      runtime = await createRuntime(config(), options());
      runtime.start();
      expect((await runtime.providers.choices("owner")).map((choice) => choice.modelId)).toEqual(["qwen3-coder:cloud", "llava:cloud"]);
      expect(await runtime.providers.choices("other")).toEqual([]);
      expect(fetchStub.mock.calls.length).toBe(networkCalls);
    } finally {
      await runtime.close();
      rmSync(directory, { recursive: true, force: true });
    }
  });
});
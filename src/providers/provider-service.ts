import { envApiKeyAuth, type AuthEvent, type AuthPrompt, type AuthType, type CredentialStore } from "@earendil-works/pi-ai";
import { createModels, getSupportedThinkingLevels, type Models, type Provider } from "@earendil-works/pi-ai/models";
import { builtinProviders } from "@earendil-works/pi-ai/providers/all";
import type { ModelRef } from "@earendil-works/pi-durable";
import { randomUUID } from "node:crypto";
import { AgentError } from "../agents/agent-input.js";
import type { EncryptionKeySetting } from "../config.js";
import type { ApplicationDatabase } from "../db/shared-database.js";
import { hvInstallation } from "../db/provider-schema.js";
import { SqliteCredentials } from "./credential-store.js";

export interface ModelChoice extends ModelRef {
  name: string; providerName: string; thinkingLevels: string[];
  /** Context window in tokens; USD per million input/output tokens. */
  contextWindow: number; inputCost: number; outputCost: number; reasoning: boolean;
}
export interface ProviderSummary {
  id: string; name: string; connected: boolean; configured: boolean;
  authType?: AuthType; methods: { type: AuthType; label: string }[];
}
export interface ProviderSettings { storageReady: boolean; providers: ProviderSummary[]; models: ModelChoice[]; activeLogin?: LoginView }
type WithoutSignal<T> = T extends unknown ? Omit<T, "signal"> : never;
type BrowserPrompt = WithoutSignal<AuthPrompt>;
export interface LoginView {
  id: string; providerId: string; state: "pending" | "complete" | "failed" | "cancelled";
  events: AuthEvent[]; prompt?: { id: string; input: BrowserPrompt }; error?: string;
}
interface LoginAttempt {
  userId: string; type: AuthType; view: LoginView; controller: AbortController;
  answer?: (value: string) => void; timer: ReturnType<typeof setTimeout>; done: Promise<void>;
}
interface UserModels { models: Models; credentials: SqliteCredentials }
export interface ProviderServiceOptions { providers?: () => Provider[] }

/** App-owned credentials/catalogs. No ambient server credentials or files are
 * inherited by a user's collection. A provider connection belongs to that user. */
export class ProviderService {
  private readonly users = new Map<string, Promise<UserModels>>();
  private readonly attempts = new Map<string, LoginAttempt>();
  private stopping = false;
  private readonly shutdown = new AbortController();
  private deviceId = "";
  onChanged: (userId: string) => Promise<void> = async () => {};
  constructor(private readonly db: ApplicationDatabase, private readonly key: EncryptionKeySetting | undefined, private readonly options: ProviderServiceOptions = {}) {}
  async initialize() {
    await this.db.insert(hvInstallation).values({ id: 1, deviceId: randomUUID() }).onConflictDoNothing().run();
    this.deviceId = (await this.db.select().from(hvInstallation).get())!.deviceId;
  }
  private admit() { if (this.stopping) throw new AgentError("Provider service is shutting down", 503); }
  private user(userId: string): Promise<UserModels> {
    this.admit();
    let pending = this.users.get(userId);
    if (!pending) {
      pending = (async () => {
        const credentials = new SqliteCredentials(this.db, userId, this.key);
        const models = createModels({ credentials, modelsStore: credentials.catalogStore(), authContext: {
          env: async () => undefined, fileExists: async () => false,
        } });
        for (let provider of (this.options.providers ?? builtinProviders)()) {
          // A web user must never select a server AWS profile/credential chain
          // or a local Google service-account file. These built-ins still work
          // with user-supplied bearer/API keys, using their native wire APIs.
          if (provider.id === "amazon-bedrock" || provider.id === "google-vertex") {
            provider = { ...provider, auth: { ...provider.auth, apiKey: envApiKeyAuth(
              provider.id === "amazon-bedrock" ? "Bedrock bearer token" : "Google Cloud API key", [],
            ) } };
          }
          models.setProvider(provider);
        }
        await models.refresh({ allowNetwork: false });
        return { models, credentials };
      })();
      this.users.set(userId, pending);
      void pending.catch(() => { if (this.users.get(userId) === pending) this.users.delete(userId); });
    }
    return pending;
  }
  async models(userId: string) { return (await this.user(userId)).models; }
  private async available(models: Models, credentials: CredentialStore) {
    const connected = await credentials.list();
    const names = new Map(models.getProviders().map((provider) => [provider.id, provider.name]));
    const result: ModelChoice[] = [];
    for (const { providerId } of connected) {
      try {
        for (const model of await models.getAvailable(providerId)) result.push({
          provider: model.provider, modelId: model.id, name: model.name, providerName: names.get(model.provider) ?? model.provider,
          thinkingLevels: getSupportedThinkingLevels(model), contextWindow: model.contextWindow, reasoning: model.reasoning,
          inputCost: model.cost.input, outputCost: model.cost.output,
        });
      } catch { /* A broken connection remains visible, but cannot admit work. */ }
    }
    return result;
  }
  async choices(userId: string) {
    const { models, credentials } = await this.user(userId);
    return this.available(models, credentials);
  }
  async validate(userId: string, ref?: ModelRef): Promise<ModelRef> {
    const choices = await this.choices(userId);
    if (!choices.length) throw new AgentError("Connect a provider on the Providers page first.", 409);
    if (!ref || !choices.some((model) => model.provider === ref.provider && model.modelId === ref.modelId)) {
      throw new AgentError("Select an available provider and model.", 409);
    }
    return { provider: ref.provider, modelId: ref.modelId };
  }
  async configured(userId: string, ref: ModelRef) {
    const { models, credentials } = await this.user(userId);
    if (!(await credentials.list()).some((entry) => entry.providerId === ref.provider)) return false;
    try { return (await models.getAvailable(ref.provider)).some((model) => model.id === ref.modelId); }
    catch { return false; }
  }
  async settings(userId: string): Promise<ProviderSettings> {
    const { models, credentials } = await this.user(userId);
    const connected = await credentials.list();
    const summaries = await Promise.all(models.getProviders().map(async (provider): Promise<ProviderSummary> => {
      const credential = connected.find((entry) => entry.providerId === provider.id);
      let configured = false;
      if (credential) {
        try { configured = Boolean(await models.checkAuth(provider.id)); } catch { /* sanitized status */ }
      }
      return {
        id: provider.id, name: provider.name, connected: Boolean(credential), configured, authType: credential?.type,
        methods: [
          ...(provider.auth.apiKey?.login ? [{ type: "api_key" as const, label: provider.auth.apiKey.name }] : []),
          ...(provider.auth.oauth ? [{ type: "oauth" as const, label: provider.auth.oauth.loginLabel ?? provider.auth.oauth.name }] : []),
        ],
      };
    }));
    const active = [...this.attempts.values()].find((attempt) => attempt.userId === userId && attempt.view.state === "pending");
    return { storageReady: Boolean(this.key), providers: summaries, models: await this.available(models, credentials), activeLogin: active ? this.view(active) : undefined };
  }
  async refresh(userId: string) {
    const { models, credentials } = await this.user(userId);
    const result = await models.refresh({ providers: (await credentials.list()).map((entry) => entry.providerId), force: true, signal: AbortSignal.any([this.shutdown.signal, AbortSignal.timeout(20_000)]) });
    await this.onChanged(userId);
    if (result.aborted || result.errors.size) throw new AgentError("Some model catalogs could not be refreshed. Previous models were kept.");
  }
  async disconnect(userId: string, providerId: string) {
    const { models } = await this.user(userId);
    for (const attempt of this.attempts.values()) {
      if (attempt.userId === userId && attempt.view.providerId === providerId && attempt.view.state === "pending") this.cancelAttempt(attempt);
    }
    await Promise.all([...this.attempts.values()].filter((attempt) => attempt.userId === userId && attempt.view.providerId === providerId).map((attempt) => attempt.done));
    await models.logout(providerId);
    await this.onChanged(userId);
  }
  async begin(userId: string, providerId: string, type: AuthType): Promise<LoginView> {
    const { models, credentials } = await this.user(userId);
    credentials.requireKey();
    const provider = models.getProvider(providerId);
    if (!provider || !(type === "oauth" ? provider.auth.oauth : provider.auth.apiKey?.login)) throw new AgentError("This provider does not support that login method.");
    for (const [oldId, previous] of this.attempts) {
      if (previous.userId === userId && previous.view.state !== "pending") {
        await previous.done;
        clearTimeout(previous.timer);
        this.attempts.delete(oldId);
      }
    }
    this.admit();
    // Check after cleanup awaits; reservation below has no intervening await.
    // Built-in OAuth flows may own fixed loopback callback ports, so serialize
    // OAuth sign-ins across users as well as all sign-ins within one user.
    for (const attempt of this.attempts.values()) {
      if (attempt.view.state === "pending" && (attempt.userId === userId || (type === "oauth" && attempt.type === "oauth"))) {
        throw new AgentError("Another provider sign-in is in progress. Finish or cancel it first.", 409);
      }
    }
    const id = randomUUID();
    const controller = new AbortController();
    const attempt: LoginAttempt = {
      userId, type, controller, view: { id, providerId, state: "pending", events: [] }, done: Promise.resolve(),
      timer: setTimeout(() => { this.cancelAttempt(attempt); this.attempts.delete(id); }, 10 * 60_000),
    };
    attempt.timer.unref();
    this.attempts.set(id, attempt);
    attempt.done = models.login(providerId, type, {
      signal: controller.signal,
      notify: (event) => {
        if (attempt.view.state !== "pending") return;
        attempt.view.events = [...attempt.view.events.slice(-19), event];
      },
      prompt: (input) => new Promise<string>((resolve, reject) => {
        const signal = AbortSignal.any([controller.signal, ...(input.signal ? [input.signal] : [])]);
        const promptId = randomUUID();
        const browserInput = { ...input };
        delete browserInput.signal;
        const cleanup = () => {
          signal.removeEventListener("abort", abort);
          if (attempt.view.prompt?.id === promptId) { delete attempt.view.prompt; delete attempt.answer; }
        };
        const abort = () => { cleanup(); reject(new Error("Login prompt cancelled")); };
        attempt.view.prompt = { id: promptId, input: browserInput };
        attempt.answer = (value) => { cleanup(); resolve(value); };
        signal.addEventListener("abort", abort, { once: true });
        if (signal.aborted) abort();
      }),
    }, { getDeviceId: () => this.deviceId, agentName: "HyperVibes" }).then(async () => {
      if (controller.signal.aborted) return;
      await models.refresh({ providers: [providerId], signal: AbortSignal.any([controller.signal, AbortSignal.timeout(20_000)]) });
      attempt.view.state = "complete";
      delete attempt.view.prompt;
      await this.onChanged(userId);
    }).catch(() => {
      if (attempt.view.state === "pending") {
        attempt.view.state = "failed";
        attempt.view.error = "Provider sign-in failed. Check your credentials and try again. For OAuth, check that its callback port is available.";
      }
      delete attempt.view.prompt;
      delete attempt.answer;
    });
    return this.view(attempt);
  }
  private owned(userId: string, id: string) {
    this.admit();
    const attempt = this.attempts.get(id);
    if (!attempt || attempt.userId !== userId) throw new AgentError("Provider sign-in not found or expired.", 404);
    return attempt;
  }
  private view(attempt: LoginAttempt): LoginView { return structuredClone(attempt.view); }
  status(userId: string, id: string) { return this.view(this.owned(userId, id)); }
  answer(userId: string, id: string, promptId: string, value: string) {
    const attempt = this.owned(userId, id);
    if (attempt.view.state !== "pending" || attempt.view.prompt?.id !== promptId || !attempt.answer) throw new AgentError("This sign-in prompt has expired.", 409);
    const input = attempt.view.prompt.input;
    // Text prompts can intentionally accept blank (e.g. Copilot's default
    // github.com domain). Secret/code/select prompts always require a value.
    if (value.length > 16_000 || (input.type !== "text" && !value.trim())) throw new AgentError("Enter a valid sign-in response (maximum 16000 characters).");
    if (input.type === "select" && !input.options.some((option) => option.id === value.trim())) throw new AgentError("Select one of the sign-in options.");
    // Copilot's native enterprise prompt otherwise permits arbitrary backend
    // network destinations. Custom endpoint configuration is not in this slice.
    if (attempt.view.providerId === "github-copilot" && input.type === "text" && value.trim() && value.trim() !== "github.com") {
      throw new AgentError("Leave the domain blank for github.com. Custom enterprise endpoints are not supported yet.");
    }
    attempt.answer(value.trim());
    return this.view(attempt);
  }
  private cancelAttempt(attempt: LoginAttempt) {
    if (attempt.view.state === "pending") {
      attempt.view.state = "cancelled";
      attempt.controller.abort();
      delete attempt.view.prompt;
      delete attempt.answer;
    }
  }
  async cancel(userId: string, id: string) {
    const attempt = this.owned(userId, id);
    this.cancelAttempt(attempt);
    await attempt.done;
  }
  async close() {
    this.stopping = true;
    this.shutdown.abort();
    for (const attempt of this.attempts.values()) { clearTimeout(attempt.timer); this.cancelAttempt(attempt); }
    await Promise.all([...this.attempts.values()].map((attempt) => attempt.done));
    for (const user of this.users.values()) await (await user).credentials.drain();
    this.attempts.clear();
    this.users.clear();
  }
}

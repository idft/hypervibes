import { type Credential, envApiKeyAuth, type Model, type OpenAICompletionsCompat } from "@earendil-works/pi-ai";
import { openAICompletionsApi } from "@earendil-works/pi-ai/api/openai-completions.lazy";
import { createProvider, type Provider, type RefreshModelsContext } from "@earendil-works/pi-ai/models";

/** Ollama Cloud (https://ollama.com) is an OpenAI-compatible API with a purely
 * dynamic catalog: no model list ships with the app. The catalog is fetched
 * live per user, once that user has connected an API key. */

const PROVIDER_ID = "ollama-cloud";
const CLOUD_BASE = "https://ollama.com";
const API_BASE = `${CLOUD_BASE}/v1`;
const FETCH_TIMEOUT_MS = 10_000;
const SHOW_CONCURRENCY = 8;
const DEFAULT_CONTEXT_WINDOW = 128_000;
// /api/show does not expose an output limit; this matches Ollama Cloud's
// observed cap for the largest responses.
const DEFAULT_MAX_TOKENS = 32_768;

/** Compat overrides for ollama.com. pi-ai detects generic-OpenAI defaults from
 * the base URL, which are wrong for Ollama (developer role, store field,
 * max_completion_tokens, prompt cache retention). */
const compat: OpenAICompletionsCompat = {
  // Ollama serves "system" messages, not the "developer" role.
  supportsDeveloperRole: false,
  supportsStore: false,
  maxTokensField: "max_tokens",
  supportsReasoningEffort: true,
  supportsUsageInStreaming: true,
  thinkingFormat: "openai",
  sendSessionAffinityHeaders: false,
  supportsLongCacheRetention: false,
};

/** Response of POST /api/show for one model. */
interface ShowDetails {
  capabilities: string[];
  modelInfo: Record<string, unknown>;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function authHeaders(credential: Credential | undefined): Record<string, string> {
  return credential?.type === "api_key" && credential.key ? { Authorization: `Bearer ${credential.key}` } : {};
}

async function fetchJson(url: string, init: RequestInit, signal: AbortSignal): Promise<unknown> {
  const response = await fetch(url, { ...init, signal: AbortSignal.any([signal, AbortSignal.timeout(FETCH_TIMEOUT_MS)]) });
  if (!response.ok) {
    if (response.status === 429) throw new Error("Ollama Cloud rate limited the request. Try again shortly.");
    throw new Error(`Ollama Cloud request to ${new URL(url).pathname} failed: ${response.status}`);
  }
  try {
    return await response.json();
  } catch {
    throw new Error(`Ollama Cloud returned invalid JSON from ${new URL(url).pathname}`);
  }
}

async function fetchModelIds(credential: Credential | undefined, signal: AbortSignal): Promise<string[]> {
  const body = await fetchJson(`${API_BASE}/models`, { headers: authHeaders(credential) }, signal);
  if (!isRecord(body) || !Array.isArray(body.data)) throw new Error("Ollama Cloud model list has an unexpected shape.");
  const ids: string[] = [];
  for (const entry of body.data) {
    if (!isRecord(entry) || typeof entry.id !== "string" || !entry.id.trim()) {
      throw new Error("Ollama Cloud model list has an unexpected shape.");
    }
    ids.push(entry.id);
  }
  return ids;
}

async function fetchModelDetails(id: string, credential: Credential | undefined, signal: AbortSignal): Promise<ShowDetails> {
  const body = await fetchJson(`${CLOUD_BASE}/api/show`, {
    method: "POST",
    headers: { ...authHeaders(credential), "Content-Type": "application/json" },
    body: JSON.stringify({ model: id }),
  }, signal);
  if (!isRecord(body) || !Array.isArray(body.capabilities) || !body.capabilities.every((capability) => typeof capability === "string")) {
    throw new Error(`Ollama Cloud returned invalid details for ${id}.`);
  }
  return { capabilities: body.capabilities, modelInfo: isRecord(body.model_info) ? body.model_info : {} };
}

/** /api/show reports context lengths as `<family>.context_length` keys. */
function contextLength(modelInfo: Record<string, unknown>): number {
  for (const [key, value] of Object.entries(modelInfo)) {
    if (key.endsWith(".context_length") && typeof value === "number" && Number.isInteger(value) && value > 0) return value;
  }
  return DEFAULT_CONTEXT_WINDOW;
}

function assembleModel(id: string, details: ShowDetails): Model<"openai-completions"> {
  return {
    id,
    name: id,
    provider: PROVIDER_ID,
    api: "openai-completions",
    baseUrl: API_BASE,
    input: details.capabilities.includes("vision") ? ["text", "image"] : ["text"],
    reasoning: details.capabilities.includes("thinking"),
    // Ollama Cloud is subscription-billed; zero rates keep usage accounting
    // from inventing pay-as-you-go charges that do not apply.
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: contextLength(details.modelInfo),
    maxTokens: DEFAULT_MAX_TOKENS,
    compat,
  };
}

/** Fetch the live catalog with the user's credential. Throws on total or
 * partial failure so the caller keeps the previous catalog instead of
 * persisting an incomplete one. Agents need tool support, so models without
 * the "tools" capability are dropped; an empty remainder is an error. */
export async function fetchModels(context: RefreshModelsContext): Promise<Model<"openai-completions">[]> {
  const credential = context.credential;
  const ids = await fetchModelIds(credential, context.signal);
  if (context.signal.aborted) return [];
  const details = new Map<string, ShowDetails>();
  const failures: string[] = [];
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(SHOW_CONCURRENCY, ids.length) }, async () => {
    while (next < ids.length) {
      const id = ids[next++]!;
      try {
        details.set(id, await fetchModelDetails(id, credential, context.signal));
      } catch (error) {
        if (context.signal.aborted) return;
        failures.push(error instanceof Error ? error.message : String(error));
      }
    }
  }));
  if (context.signal.aborted) return [];
  if (failures.length) {
    throw new Error(`Ollama Cloud catalog refresh incomplete: ${failures.length} of ${ids.length} model(s) failed (${failures[0]})`);
  }
  const models = [...details].filter(([, shown]) => shown.capabilities.includes("tools"))
    .map(([id, shown]) => assembleModel(id, shown));
  if (!models.length) throw new Error("Ollama Cloud catalog contains no models with tool support");
  return models;
}

/** The Ollama Cloud provider. Its catalog starts empty and is populated from
 * ollama.com on refresh with the user's stored API key. */
export function ollamaCloudProvider(): Provider<"openai-completions"> {
  return createProvider({
    id: PROVIDER_ID,
    name: "Ollama Cloud",
    baseUrl: API_BASE,
    // No ambient server credential: only a per-user stored key resolves.
    auth: { apiKey: envApiKeyAuth("Ollama Cloud API key", []) },
    models: [],
    api: openAICompletionsApi(),
    fetchModels,
  });
}
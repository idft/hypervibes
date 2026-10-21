import { isRedirect } from "@tanstack/react-router";
import { QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ models: vi.fn(), accounts: vi.fn() }));
vi.mock("./providers.js", () => ({ getModelChoices: mocks.models }));
vi.mock("./agents.js", () => ({ getTradingAccountChoices: mocks.accounts, createAgent: vi.fn(), createAgentSubaccount: vi.fn() }));
import { Route } from "../routes/_authenticated.agents.new.js";
import { modelChoicesQueryOptions } from "./provider-queries.js";

const routeLoader = Route.options.loader as (options: { context: { queryClient: QueryClient } }) => Promise<unknown>;
const loader = () => routeLoader({ context: { queryClient: new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity } } }) } });
describe("provider setup creation guard", () => {
  beforeEach(() => vi.resetAllMocks());
  it("redirects new-agent navigation before any exchange request when no models are available", async () => {
    mocks.models.mockResolvedValue([]);
    const result = await loader().catch((error: unknown) => error);
    expect(isRedirect(result)).toBe(true);
    if (isRedirect(result)) expect(result.options.to).toBe("/providers");
    expect(mocks.accounts).not.toHaveBeenCalled();
  });
  it("refreshes an invalidated empty catalog before deciding whether to redirect", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity, staleTime: Infinity } } });
    client.setQueryData(modelChoicesQueryOptions().queryKey, []);
    await client.invalidateQueries({ queryKey: ["providers"] });
    mocks.models.mockResolvedValue([{ provider: "test", modelId: "model" }]);
    mocks.accounts.mockResolvedValue({ mainAddress: "fixture" });
    await routeLoader({ context: { queryClient: client } });
    expect(mocks.models).toHaveBeenCalledOnce(); expect(mocks.accounts).toHaveBeenCalledOnce();
    client.clear();
  });
  it("loads choices when a connected provider has a model", async () => {
    const models = [{ provider: "test", modelId: "model", name: "Model", providerName: "Test", thinkingLevels: [], contextWindow: 200_000, inputCost: 3, outputCost: 15, reasoning: true }];
    mocks.models.mockResolvedValue(models); mocks.accounts.mockResolvedValue({ mainAddress: "fixture" });
    expect(await loader()).toBeUndefined();
    expect(mocks.accounts).toHaveBeenCalledOnce();
  });
});

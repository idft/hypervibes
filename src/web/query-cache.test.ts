import { QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ list: vi.fn(), refresh: vi.fn(), settings: vi.fn(), models: vi.fn() }));
vi.mock("./agents.js", () => ({ listAgents: mocks.list }));
vi.mock("./providers.js", () => ({ getProviderSettings: mocks.settings, getModelChoices: mocks.models, refreshProviderModels: mocks.refresh }));
import { agentsQueryOptions, invalidateAgentCollection } from "./agent-queries.js";
import { modelChoicesQueryOptions, providerMutationOptions, providerSettingsQueryOptions } from "./provider-queries.js";

describe("shared application query caches", () => {
  let client: QueryClient;
  beforeEach(() => {
    vi.resetAllMocks();
    client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity, staleTime: 30_000 } } });
  });
  afterEach(() => client.clear());
  it("deduplicates layout and index loaders on the same agent list entry", async () => {
    const agents = [{ id: "a", name: "Desk" }];
    mocks.list.mockResolvedValue(agents);
    const [layout, index] = await Promise.all([client.ensureQueryData(agentsQueryOptions()), client.ensureQueryData(agentsQueryOptions())]);
    expect(layout).toEqual(agents); expect(index).toEqual(agents);
    expect(mocks.list).toHaveBeenCalledOnce();
    await client.ensureQueryData(agentsQueryOptions());
    expect(mocks.list).toHaveBeenCalledOnce();
  });
  it("invalidates inactive catalogs and onboarding even when a refresh partially fails", async () => {
    const keys = [providerSettingsQueryOptions().queryKey, modelChoicesQueryOptions().queryKey, ["setup"]];
    keys.forEach((key) => client.setQueryData(key, []));
    client.setQueryData(["journal", "other", "activity", 1], { linked: true });
    mocks.refresh.mockRejectedValue(new Error("Some model catalogs could not be refreshed. Previous models were kept."));
    const mutation = client.getMutationCache().build(client, providerMutationOptions(client));
    await expect(mutation.execute({ type: "refresh" })).rejects.toThrow("Previous models were kept");
    keys.forEach((key) => expect(client.getQueryState(key)?.isInvalidated).toBe(true));
    expect(client.getQueryState(["journal", "other", "activity", 1])?.isInvalidated).toBe(false);
  });
  it("invalidates account bindings and onboarding together after account/agent mutations", async () => {
    const keys = [["agents"], ["trading-accounts"], ["account"], ["setup"]];
    keys.forEach((key) => client.setQueryData(key, []));
    client.setQueryData(["providers", "models"], []);
    await invalidateAgentCollection(client);
    keys.forEach((key) => expect(client.getQueryState(key)?.isInvalidated).toBe(true));
    expect(client.getQueryState(["providers", "models"])?.isInvalidated).toBe(false);
  });
});

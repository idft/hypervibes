import { type QueryClient, QueryObserver } from "@tanstack/react-query";
import { createTestQueryClient } from "../test/query.js";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { templateDraft } from "../agents/subagent-templates.js";
import { workerTriggers } from "../agents/worker-triggers.js";
import { runPagination } from "../agents/subagent-history.js";
import type { SubagentDetail } from "./subagent-queries.js";
import type { SubagentRow } from "../db/subagent-schema.js";
const mocks = vi.hoisted(() => ({ list: vi.fn(), detail: vi.fn(), dashboard: vi.fn(), run: vi.fn(), saveSection: vi.fn(), start: vi.fn(), stop: vi.fn() }));
vi.mock("./subagents.js", () => ({ listSubagents: mocks.list, getSubagent: mocks.detail, getDashboardSubagents: mocks.dashboard, getSubagentRun: mocks.run, saveSubagentSection: mocks.saveSection, runSubagent: mocks.start, stopSubagentRun: mocks.stop }));
import { dashboardSubagentsQueryOptions, subagentQueryOptions, subagentsQueryOptions } from "./subagent-queries.js";
import { subagentMutations } from "./subagent-mutations.js";
import { memoriesQueryOptions, insightsQueryOptions } from "./memory-queries.js";
import { artifactQueryOptions } from "./artifact-queries.js";
import { chartHistoryQueryOptions } from "./chart-queries.js";

describe("domain query ownership", () => {
  let client: QueryClient;
  beforeEach(() => {
    vi.resetAllMocks();
    client = createTestQueryClient({ staleTime: Infinity });
  });
  it("updates saved definitions in both caches without replacing detail history or fetching SSE seeds", async () => {
    const draft = templateDraft("basic-analysis", { provider: "test", modelId: "model", thinkingLevel: "" });
    const row: SubagentRow = { ...draft, triggers: workerTriggers(draft.triggers), id: "worker", agentId: "agent", kind: "worker", archived: false, conversationId: null, nextRunAt: null, templateOrigin: null, createdAt: 1, updatedAt: 1 };
    const detailKey = subagentQueryOptions("agent", row.id).queryKey;
    const listKey = subagentsQueryOptions("agent").queryKey;
    const history: SubagentDetail = { definition: row, runs: [], pagination: runPagination(0), workers: [{ id: "other", name: "Other", archived: true, tools: [] }], schedulerError: null };
    client.setQueryData(detailKey, history);
    client.setQueryData(listKey, [row]);
    client.setQueryData(dashboardSubagentsQueryOptions("agent").queryKey, []);
    client.setQueryData(subagentQueryOptions("other-agent", row.id).queryKey, history);
    // A mounted dashboard must continue to receive its stream, not another GET.
    const observer = new QueryObserver(client, dashboardSubagentsQueryOptions("agent"));
    const unsubscribe = observer.subscribe(() => {});
    try {
      const saved = { ...row, enabled: true, updatedAt: 2 };
      mocks.saveSection.mockResolvedValue(saved);
      const mutation = client.getMutationCache().build(client, { mutationFn: subagentMutations(client, "agent").saveSection<"enabled"> });
      await expect(mutation.execute({ id: row.id, section: "enabled", settings: { enabled: true } })).resolves.toEqual(saved);
      expect(mocks.saveSection).toHaveBeenCalledWith({ data: { agentId: "agent", id: row.id, section: "enabled", settings: { enabled: true } } });
      expect(client.getQueryData(detailKey)).toEqual({ ...history, definition: saved });
      expect(client.getQueryData(listKey)).toEqual([saved]);
      expect(client.getQueryData(subagentQueryOptions("other-agent", row.id).queryKey)).toEqual(history);
      expect(mocks.detail).not.toHaveBeenCalled(); expect(mocks.dashboard).not.toHaveBeenCalled();
    } finally { unsubscribe(); }
  });
  it.each(["run", "stop"] as const)("%s invalidates only the target agent's list, detail and live seed caches", async (action) => {
    const keys = [["subagents", "agent", "list"], ["subagents", "agent", "detail", "worker"], ["subagents", "agent", "dashboard"], ["agent-chat", "agent"]];
    keys.forEach((key) => client.setQueryData(key, []));
    client.setQueryData(["subagents", "other-agent", "list"], []);
    const mutations = subagentMutations(client, "agent");
    if (action === "run") {
      mocks.start.mockResolvedValue({ id: "run" });
      const input = { id: "worker", task: "Research", requestId: "stable-retry-id" };
      await expect(mutations.run(input)).resolves.toEqual({ id: "run" });
      expect(mocks.start).toHaveBeenCalledWith({ data: { agentId: "agent", ...input } });
    } else {
      await mutations.stop("run");
      expect(mocks.stop).toHaveBeenCalledWith({ data: { agentId: "agent", runId: "run" } });
    }
    keys.forEach((key) => expect(client.getQueryState(key)?.isInvalidated).toBe(true));
    expect(client.getQueryState(["subagents", "other-agent", "list"])?.isInvalidated).toBe(false);
  });
  it.each(["saveSection", "run", "stop"] as const)("failed %s preserves cached state and propagates the server error", async (action) => {
    const key = subagentsQueryOptions("agent").queryKey;
    client.setQueryData(key, []);
    const mutations = subagentMutations(client, "agent");
    const failure = new Error("Operation rejected");
    let operation: () => Promise<unknown>;
    if (action === "saveSection") {
      mocks.saveSection.mockRejectedValue(failure);
      operation = () => mutations.saveSection({ id: "worker", section: "enabled", settings: { enabled: true } });
    } else if (action === "run") {
      mocks.start.mockRejectedValue(failure);
      operation = () => mutations.run({ id: "worker", task: "Research", requestId: "stable-retry-id" });
    } else {
      mocks.stop.mockRejectedValue(failure);
      operation = () => mutations.stop("run");
    }
    await expect(operation()).rejects.toBe(failure);
    expect(client.getQueryData(key)).toEqual([]);
    expect(client.getQueryState(key)?.isInvalidated).toBe(false);
  });
  it("separates pages, filters, scopes, agents and artifact portions in cache keys", () => {
    const keys: readonly (readonly unknown[])[] = [
      memoriesQueryOptions("agent").queryKey,
      memoriesQueryOptions("agent", { instrument: "BTC" }).queryKey,
      memoriesQueryOptions("agent", { cursor: { ref: "m1", createdAt: 1000 } }).queryKey,
      memoriesQueryOptions("other-agent").queryKey,
      insightsQueryOptions("agent").queryKey,
      insightsQueryOptions("agent", "worker").queryKey,
      insightsQueryOptions("agent", "worker", 2).queryKey,
      artifactQueryOptions("agent", "artifact").queryKey,
      artifactQueryOptions("agent", "artifact", 100).queryKey,
      artifactQueryOptions("other-agent", "artifact").queryKey,
      chartHistoryQueryOptions("agent", "BTC", "1h").queryKey,
      chartHistoryQueryOptions("agent", "ETH", "1h").queryKey,
      chartHistoryQueryOptions("agent", "BTC", "4h", 1000).queryKey,
    ];
    keys.forEach((key, index) => client.setQueryData(key, index));
    expect(client.getQueryCache().getAll()).toHaveLength(keys.length);
    keys.forEach((key, index) => expect(client.getQueryData(key)).toBe(index));
  });
});

import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { Type } from "@earendil-works/pi-ai";
import { HttpTransport } from "@nktkas/hyperliquid";
import { loadAccountStatus } from "../account-status.js";
import { createHyperliquidExtension } from "../durable-extension.js";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { createRegistry, defineExtension, defineTool, Harness, MemoryStorage, type NestedToolExecutionResult } from "@earendil-works/pi-durable";
import { describe, expect, it, vi } from "vitest";
import { createOrderTools } from "./order-tools.js";
import type { OrderService } from "./order-service.js";
import type { OperationResult } from "./order-contracts.js";

const outcome: OperationResult = { operationId: "operation", requestId: "logical-request", kind: "submit", results: [], uncertain: false, warnings: [] };

describe("Durable order tool contracts for nested/code-mode calls", () => {
  it("returns schema-validated structured outputs and reuses a keyed nested call", async () => {
    const submit = vi.fn(async () => outcome);
    const list = vi.fn(async () => ({ items: [], source: "open", observedAt: 1, nextOffset: null, coverage: "current snapshot" }));
    const service = { submit, list, executionHandle: async () => undefined } as unknown as OrderService;
    const registry = createRegistry();
    registry.install(createHyperliquidExtension({ agentAccountStatus: async () => loadAccountStatus(new HttpTransport(), null), marketSnapshot: async () => { throw new Error("Not used in order tests"); } }, "owner", "agent", service));
    const results: NestedToolExecutionResult[] = [];
    registry.install(defineExtension({ name: "program", tools: [defineTool({
      name: "program", parameters: Type.Object({}), description: "Programmatic tool caller, like code mode", replay: "safe",
      execute: async (_args, api, ctx) => {
        const args = { request_id: "logical-request", orders: [{ symbol: "BTC", side: "buy", order_type: "limit", size: "0.001", price: "100000" }] };
        results.push(await api.executeTool("submit_orders", args, ctx, { key: "place" }));
        results.push(await api.executeTool("submit_orders", args, ctx, { key: "place" }));
        results.push(await api.executeTool("list_orders", {}, ctx));
        results.push(await api.executeTool("submit_orders", { ...args, orders: [{ ...args.orders[0], size: 0.001 }] }, ctx));
        results.push(await api.executeTool("get_account", {}, ctx));
        return { output: [{ type: "text", text: "program complete" }] };
      },
    })] }));
    const models = createModels(); const faux = fauxProvider(); models.setProvider(faux.provider);
    faux.setResponses([fauxAssistantMessage(fauxToolCall("program", {}), { stopReason: "toolUse" }), fauxAssistantMessage("Done")]);
    const harness = await Harness.open(new MemoryStorage(), { models, registry }, context);
    try {
      const root = await harness.root(context, { agent: { model: { provider: "faux", modelId: "faux-1" }, modelTools: [(registry.snapshot().extension("program")!.tools!)[0]!] } });
      const settled = await (await root.submit({ type: "input", content: "Run program" }, context)).wait(context);
      expect(settled.status).toBe("done");
      expect(results).toHaveLength(5);
      expect(results[0]).toMatchObject({ isError: false, structuredOutput: outcome });
      expect(results[1]!.taskId).toBe(results[0]!.taskId);
      expect(results[2]).toMatchObject({ isError: false, structuredOutput: { items: [], source: "open" } });
      expect(results[3]!.isError).toBe(true);
      expect(results[4]).toMatchObject({ isError: false, structuredOutput: { address: null, spotUsdc: null, sections: { spot: { fresh: false } } } });
      expect(submit).toHaveBeenCalledOnce();
      expect(submit).toHaveBeenCalledWith("owner", "agent", expect.objectContaining({ request_id: "logical-request" }), expect.any(AbortSignal), undefined);
      expect(list).toHaveBeenCalledWith("owner", "agent", {}, expect.any(AbortSignal));
    } finally { await harness.close(context); }
  });

  it("distinguishes exchange request IDs from worker inbox and assignment IDs", () => {
    const tool = createOrderTools({} as OrderService, "owner", "agent").find((tool) => tool.name === "get_order_request")!;
    expect(tool.description).toContain("original request_id used with submit_orders, cancel_orders or cancel_all_orders");
    expect(tool.description).toContain("Worker inbox request IDs, assignment/run IDs and exchange order OIDs are NOT order request IDs");
    expect(tool.description).toContain("list_orders with source=local");
    expect(tool.description).toContain("Not-found does not prove rejection, cancellation or absence of exchange orders");
    expect(tool.parameters).toMatchObject({ properties: { request_id: { description: expect.stringContaining("Never a worker inbox request ID") } } });
  });

  it("documents venue minimum notional without silently upsizing orders", () => {
    const tools = createOrderTools({} as OrderService, "owner", "agent");
    expect(tools.find((tool) => tool.name === "submit_orders")!.description).toContain("$10 minimum order notional");
    expect(tools.find((tool) => tool.name === "submit_orders")!.description).toContain("Do not silently increase size");
    for (const tool of tools) {
      expect(tool.structuredOutputSchema).toBeDefined();
      expect(tool.callers).not.toEqual(["model"]);
    }
  });
});

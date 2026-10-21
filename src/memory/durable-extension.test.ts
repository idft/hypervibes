import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { createRegistry, defineExtension, defineTool, Harness, MemoryStorage, type NestedToolExecutionResult } from "@earendil-works/pi-durable";
import { Type } from "@earendil-works/pi-ai";
import { describe, expect, it, vi } from "vitest";
import type { ExecutionContextService } from "../agents/execution-context.js";
import type { SharedDatabase } from "../db/shared-database.js";
import { createMemoryExtension } from "./durable-extension.js";
import { DecisionPayloadSchema, ReviewPayloadSchema } from "./tool-schemas.js";

const decision = { topic: "trade_decision", instrument: "BTC", summary: "Operator chart request", content: "The operator requested the trade based on their chart reading.", payload: { version: 1, action: "enter", direction: "long", reason: "operator_request", researchRefs: [], plan: "Operator entry with explicit TP and SL" } };

describe("memory tool guidance and validation", () => {
  it("explains optional operator decision recording and fixed schema versions", () => {
    const extension = createMemoryExtension("agent", { memories: {}, insights: {} } as ExecutionContextService, {} as SharedDatabase);
    const tool = extension.tools!.find((tool) => tool.name === "record_trade_decision")!;
    expect(tool.description).toContain("A direct/manual operator request does not require creating a decision or consulting memories");
    expect(tool.description).toContain("researchRefs: []");
    expect(tool.description).toContain("A saved decision does not prove execution");
    for (const payload of [DecisionPayloadSchema, ReviewPayloadSchema]) {
      expect(payload.properties.version).toMatchObject({ description: expect.stringContaining("NOT a decision count or revision number") });
    }
  });

  it("returns actionable errors for direct and nested calls before resolving identity or writing memories", async () => {
    const recordDecision = vi.fn(async () => ({ ref: "m1", kind: "decision", createdAt: 1, validUntil: null }));
    const resolve = vi.fn(async () => ({ actor: {} }));
    const execution = { memories: { recordDecision }, insights: {}, resolve } as unknown as ExecutionContextService;
    const registry = createRegistry();
    const memory = createMemoryExtension("agent", execution, {} as SharedDatabase);
    registry.install(memory);
    const nested: NestedToolExecutionResult[] = [];
    registry.install(defineExtension({ name: "program", tools: [defineTool({
      name: "program", description: "Nested memory caller", parameters: Type.Object({}), replay: "safe",
      execute: async (_args, api, ctx) => {
        nested.push(await api.executeTool("record_trade_decision", { ...decision, payload: { ...decision.payload, version: 2 } }, ctx));
        return { output: [{ type: "text", text: "Nested validation checked" }] };
      },
    })] }));
    const models = createModels(), faux = fauxProvider(); models.setProvider(faux.provider);
    faux.setResponses([
      fauxAssistantMessage([
        fauxToolCall("record_trade_decision", { ...decision, payload: { ...decision.payload, version: 2 } }),
        fauxToolCall("publish_review", { payload: { version: 2 } }),
        fauxToolCall("program", {}),
      ], { stopReason: "toolUse" }),
      () => {
        expect(resolve).not.toHaveBeenCalled();
        expect(recordDecision).not.toHaveBeenCalled();
        return fauxAssistantMessage(fauxToolCall("record_trade_decision", decision), { stopReason: "toolUse" });
      },
      fauxAssistantMessage("Done"),
    ]);
    const harness = await Harness.open(new MemoryStorage(), { models, registry }, context);
    try {
      const root = await harness.root(context, { agent: { model: { provider: "faux", modelId: "faux-1" }, tools: [...memory.tools!.filter((tool) => ["record_trade_decision", "publish_review"].includes(tool.name)), ...registry.snapshot().extension("program")!.tools!] } });
      expect((await (await root.submit({ type: "input", content: "Check memory validation" }, context)).wait(context)).status).toBe("done");
      const results = (await root.entries({ order: "ascending" }, 100, undefined, context)).items.flatMap((entry) => entry.model ?? []).filter((message) => message.role === "toolResult");
      const failed = results.filter((message) => message.isError);
      expect(failed).toHaveLength(2);
      for (const result of [...failed, ...nested]) {
        expect(result.isError).toBe(true);
        expect(JSON.stringify(result)).toContain("payload.version must be the number 1 (schema version, not a revision counter); received 2");
      }
      expect(nested).toHaveLength(1);
      expect(recordDecision).toHaveBeenCalledOnce();
      expect(resolve).toHaveBeenCalledOnce();
      expect(recordDecision).toHaveBeenCalledWith({}, expect.any(String), decision);
      expect(results.at(-1)).toMatchObject({ toolName: "record_trade_decision", isError: false });
    } finally { await harness.close(context); }
  });
});

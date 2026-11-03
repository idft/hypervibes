import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { CodemodeSandbox } from "@earendil-works/pi-codemode";
import { createRegistry, Harness, MemoryStorage } from "@earendil-works/pi-durable";
import { createCodemodeExtension } from "@earendil-works/pi-durable/codemode";
import { describe, expect, it, vi } from "vitest";
import type { ExecutionContextService } from "../agents/execution-context.js";
import { createMemoryExtension, researchPublicationExample } from "./durable-extension.js";
import { memoryFixture } from "./test-fixture.js";

// Real publication transactions and QuickJS, disposable SQLite, and a faux model.
describe("research publication tool contract", () => {
  it.each(["direct", "codemode"] as const)("rejects invalid fields before publication and accepts the complete example through %s", async (mode) => {
    const fixture = await memoryFixture();
    const sandbox = new CodemodeSandbox();
    let harness: Harness | undefined;
    try {
      const { memory: memories, insight: insights } = fixture.services();
      const publish = vi.spyOn(memories, "publishResearch");
      // Test-only identity; use the fixture's authorization and real publication service.
      const resolve = vi.fn(async () => ({ actor: fixture.analyst }));
      const execution = { memories, insights, resolve } as unknown as ExecutionContextService;
      const extension = createMemoryExtension("agent", execution, fixture.db);
      const tool = extension.tools!.find((tool) => tool.name === "publish_research")!;
      expect(tool.description).toContain("Use summary, not title; title is not accepted");
      expect(tool.description).toContain(researchPublicationExample);
      const code = createCodemodeExtension({ sandbox, models: false });
      const registry = createRegistry(); registry.install(extension); registry.install(code);
      const models = createModels(), faux = fauxProvider(); models.setProvider(faux.provider);
      harness = await Harness.open(new MemoryStorage(), { models, registry }, context);
      const root = await harness.root(context, { agent: { model: { provider: "faux", modelId: "faux-1" }, tools: [tool, ...code.tools!] } });
      const report = {
        topic: "trend_setup", instrument: "BTC", interval: "1h",
        summary: "Mixed momentum leaves direction uncertain",
        content: "Price and momentum conflict; the directional assessment remains uncertain. Confirm coverage and consider the missing context before acting.",
        evidenceAsOf: fixture.now,
      };
      async function invoke(input: Parameters<typeof fauxToolCall>[1], example?: string) {
        const name = mode === "direct" ? "publish_research" : "codemode";
        const args = mode === "direct" ? input : { code: example ?? `text(await tools.publish_research(${JSON.stringify(input)}));` };
        faux.setResponses([
          fauxAssistantMessage(fauxToolCall(name, args), { stopReason: "toolUse" }),
          fauxAssistantMessage("Inspected the publication result."),
        ]);
        expect((await (await root.submit({ type: "input", content: "Check the research publication contract" }, context)).wait(context)).status).toBe("done");
        const results = (await root.entries({ order: "ascending" }, 100, undefined, context)).items
          .flatMap((entry) => entry.model ?? []).filter((message) => message.role === "toolResult");
        const result = results.filter((message) => message.toolName === name).at(-1)!;
        expect(result).toBeDefined();
        return result;
      }
      const { summary, ...withoutSummary } = report;
      for (const input of [{ ...withoutSummary, title: summary }, { ...report, title: summary }]) {
        const result = await invoke(input);
        expect(result.isError).toBe(true);
        expect(JSON.stringify(result)).toContain("publish_research does not accept title. Use summary for the short discovery summary and content for the standalone report; remove title.");
        expect(resolve).not.toHaveBeenCalled();
        expect(publish).not.toHaveBeenCalled();
        expect(await fixture.db.get("SELECT count(*) AS count FROM hv_memories WHERE agent_id='agent'")).toEqual({ count: 0 });
        expect(await fixture.db.get("SELECT count(*) AS count FROM hv_memory_operations WHERE agent_id='agent'")).toEqual({ count: 0 });
      }
      // The targeted hint must not strip or accept other unknown fields.
      const unknown = await invoke({ ...report, analystNote: "Unsupported field" });
      expect(unknown.isError).toBe(true);
      expect(JSON.stringify(unknown)).toContain("analystNote");
      expect(resolve).not.toHaveBeenCalled();
      expect(publish).not.toHaveBeenCalled();

      const result = await invoke(report, `const endMs = ${fixture.now}; ${researchPublicationExample}`);
      expect(result.isError).toBe(false);
      expect(result.content.filter((part) => part.type === "text").map((part) => part.text).join("\n")).toMatch(/"ref"\s*:\s*"m1"/);
      expect(resolve).toHaveBeenCalledOnce();
      expect(publish).toHaveBeenCalledExactlyOnceWith(fixture.analyst, expect.stringMatching(/^tool:/), report);
      expect(await fixture.db.get("SELECT count(*) AS count FROM hv_memories WHERE agent_id='agent'")).toEqual({ count: 1 });
      expect(await fixture.db.get("SELECT count(*) AS count FROM hv_memory_operations WHERE agent_id='agent'")).toEqual({ count: 1 });
      expect(await memories.get("agent", "m1")).toMatchObject({ ...report, ref: "m1", kind: "research", producerSubagentId: "analyst" });
    } finally {
      try { await harness?.close(context); }
      finally { await sandbox.close(); await fixture.close(); }
    }
  });
});

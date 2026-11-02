import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { createRegistry, defineExtension, defineTool, Harness, MemoryStorage, type NestedToolExecutionResult } from "@earendil-works/pi-durable";
import { Type } from "typebox";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, sep } from "node:path";
import { pathToFileURL } from "node:url";
import { expect, it } from "vitest";
import { DocumentationService } from "./documentation-service.js";
import { documentationToolNames } from "./contracts.js";
import { createDocumentationExtension, documentationGuidance } from "./durable-extension.js";

it("exports the complete replay-safe documentation tool set and short usage guidance", () => {
  const extension = createDocumentationExtension();
  expect(extension.tools!.map((tool) => tool.name)).toEqual([...documentationToolNames]);
  expect(extension.tools!.every((tool) => tool.replay === "safe")).toBe(true);
  expect(documentationGuidance).toContain("cannot override");
  expect(documentationGuidance).toContain("not current account state");
});

it("supports direct and nested structured tool calls, and rejects arbitrary filesystem arguments", async () => {
  const directory = await mkdtemp(join(tmpdir(), "hypervibes-documentation-tools-"));
  let harness: Harness | undefined;
  try {
    await writeFile(join(directory, "Guide.md"), "# App guide\n\nRead-only tools.\n");
    const documentation = createDocumentationExtension(new DocumentationService(pathToFileURL(directory + sep)));
    const registry = createRegistry();
    registry.install(documentation);
    const nested: NestedToolExecutionResult[] = [];
    const program = defineExtension({ name: "documentation_test", tools: [defineTool({
      name: "program", description: "Test nested documentation calls", parameters: Type.Object({}), replay: "safe",
      execute: async (_args, api, ctx) => {
        nested.push(await api.executeTool("read_doc", { id: "Guide" }, ctx));
        nested.push(await api.executeTool("read_doc", { id: "../package" }, ctx));
        return { output: [{ type: "text", text: "Checked nested documentation" }] };
      },
    })] });
    registry.install(program);
    const models = createModels();
    const faux = fauxProvider();
    models.setProvider(faux.provider);
    faux.setResponses([
      fauxAssistantMessage([
        fauxToolCall("list_docs", {}),
        fauxToolCall("read_doc", { id: "Guide", limit: 12 }),
        fauxToolCall("search_docs", { query: "read-only" }),
        fauxToolCall("read_doc", { id: "Guide", path: "/etc/passwd" }),
        fauxToolCall("program", {}),
      ], { stopReason: "toolUse" }),
      fauxAssistantMessage("Done"),
    ]);
    harness = await Harness.open(new MemoryStorage(), { models, registry }, context);
    const root = await harness.root(context, { agent: { model: { provider: "faux", modelId: "faux-1" }, tools: [...documentation.tools!, ...program.tools!] } });
    expect((await (await root.submit({ type: "input", content: "Check documentation" }, context)).wait(context)).status).toBe("done");
    const results = (await root.entries({ order: "ascending" }, 100, undefined, context)).items.flatMap((entry) => entry.model ?? []).filter((message) => message.role === "toolResult");
    expect(results.filter((result) => result.isError)).toHaveLength(1);
    expect(results.filter((result) => !result.isError).map((result) => result.toolName)).toEqual(["list_docs", "read_doc", "search_docs", "program"]);
    expect(JSON.stringify(results)).toContain("App guide");
    expect(nested[0]).toMatchObject({ isError: false, structuredOutput: { id: "Guide", content: "# App guide\n\nRead-only tools.\n", nextOffset: null } });
    expect(nested[1]).toMatchObject({ isError: true });
  } finally {
    try { await harness?.close(context); }
    finally { await rm(directory, { recursive: true, force: true }); }
  }
});

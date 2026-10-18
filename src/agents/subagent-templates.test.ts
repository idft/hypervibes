import { describe, expect, it } from "vitest";
import { subagentInput, workerToolNames } from "./subagent-input.js";
import analysisManifest from "./templates/basic-analysis.json" with { type: "json" };
import tradingManifest from "./templates/basic-trading.json" with { type: "json" };
import { parseTemplateCatalog, parseTemplateManifest } from "./template-manifest.js";
import { subagentTemplates, templateDraft } from "./subagent-templates.js";
import { createMemoryExtension } from "../memory/durable-extension.js";
import { createHyperliquidExtension } from "../hyperliquid/durable-extension.js";
import { createTaExtension } from "../research/ta-extension.js";
import type { OrderService } from "../hyperliquid/orders/order-service.js";
import type { HyperliquidService } from "../hyperliquid/service.js";
import type { ExecutionContextService } from "./execution-context.js";
import type { SharedDatabase } from "../db/shared-database.js";
import type { ResearchResponder } from "../research/tool-response.js";

describe("bundled editable worker manifests", () => {
  it("loads standalone JSON documents through the same strict data parser", () => {
    expect(parseTemplateManifest(JSON.parse(JSON.stringify(analysisManifest)))).toEqual(subagentTemplates[0]);
    expect(subagentTemplates.every((template) => template.formatVersion === 1)).toBe(true);
    expect(() => parseTemplateCatalog([analysisManifest, analysisManifest])).toThrow("Duplicate");
  });
  it.each([
    { ...analysisManifest, formatVersion: 2 },
    { ...analysisManifest, provider: "template-authored-provider" },
    { ...analysisManifest, instructions: "Executable module rather than paragraphs" },
    { ...analysisManifest, tools: ["delegate_subagent"] },
    { ...analysisManifest, tools: ["invented_remote_tool"] },
    { ...analysisManifest, defaults: { ...analysisManifest.defaults, enabled: true } },
    { ...analysisManifest, unknownExtraField: true },
    { ...tradingManifest, defaults: { ...tradingManifest.defaults, persistent: true } },
  ])("rejects unsupported/unsafe manifests %#", (manifest) => {
    expect(() => parseTemplateManifest(manifest)).toThrow();
  });
  it("creates independent drafts without mutating the catalog or sharing nested state", () => {
    const model = { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" };
    const first = templateDraft("basic-trading", model);
    first.tools.length = 0;
    first.instructions = "Mutated copy";
    const second = templateDraft("basic-trading", model);
    expect(second.tools.length).toBeGreaterThan(0);
    expect(second.instructions).not.toBe("Mutated copy");
    expect(subagentInput(second).tools).toEqual(second.tools);
  });
  it("has only real permitted tools, no recursive delegation, no credentials or enabled/funded defaults", () => {
    const tools = [...createMemoryExtension("agent", {} as ExecutionContextService, {} as SharedDatabase).tools!, ...createHyperliquidExtension({} as HyperliquidService, "owner", "agent", {} as OrderService).tools!, ...createTaExtension({} as HyperliquidService["charts"], (() => { throw new Error("No execution in manifest test"); }) as ResearchResponder).tools!].map((tool) => tool.name);
    // These two are defined by the existing research extension, validated by
    // its integration tests; no remote catalog or import-time host is needed.
    tools.push("research_candles", "research_journal");
    for (const template of subagentTemplates) {
      const draft = templateDraft(template.id, { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" });
      expect(draft).toMatchObject({ enabled: false, persistent: false, completion: "notify", provider: "operator-selected", modelId: "operator-model" });
      expect(template.tools.every((tool) => (workerToolNames as readonly string[]).includes(tool) && tools.includes(tool))).toBe(true);
      expect(template.tools).not.toContain("delegate_subagent"); expect(template.tools).not.toContain("cancel_all_orders");
      expect(template.instructions).not.toMatch(/\{\{|\$\{/);
      expect(template.instructions).toContain("not a publication or order receipt");
      expect(subagentInput(draft).tools).toEqual(template.tools);
    }
    expect(subagentTemplates.find((template) => template.id === "basic-analysis")!.tools).not.toContain("submit_orders");
    expect(subagentTemplates.find((template) => template.id === "basic-trading")!.tools).not.toContain("append_learning");
    expect(subagentTemplates.find((template) => template.id === "basic-review")!.tools).not.toContain("submit_orders");
  });
  it("rejects unknown templates and starts every draft paused", () => {
    const model = { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" };
    expect(() => templateDraft("remote-catalog:arbitrary", model)).toThrow("Unknown bundled");
    for (const template of subagentTemplates) {
      const draft = templateDraft(template.id, model);
      expect(draft.enabled).toBe(false);
      expect(draft.name).toBe(template.name);
    }
  });
});
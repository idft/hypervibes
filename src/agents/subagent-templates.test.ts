import { describe, expect, it } from "vitest";
import { subagentInput, workerToolNames } from "./subagent-input.js";
import analysisManifest from "./templates/basic-analysis.json" with { type: "json" };
import tradingManifest from "./templates/basic-trading.json" with { type: "json" };
import { parseTemplateCatalog, parseTemplateManifest } from "./template-manifest.js";
import { workflowDraft, subagentTemplates, templateDraft } from "./subagent-templates.js";
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
    { ...analysisManifest, purpose: "trading" },
    { ...analysisManifest, workflow: { ...analysisManifest.workflow, strategy: "" } },
    { ...tradingManifest, workflow: { ...tradingManifest.workflow, automaticEntriesEnabled: true } },
  ])("rejects unsupported/unsafe manifests %#", (manifest) => {
    expect(() => parseTemplateManifest(manifest)).toThrow();
  });
  it("creates independent drafts without mutating the catalog or sharing nested workflow state", () => {
    const model = { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" };
    const first = templateDraft("basic-trading", model);
    if (first.workflow?.purpose !== "trading") throw new Error("Missing trading draft");
    first.workflow.instruments.push("BTC");
    first.workflow.baseSizeByInstrument.BTC = "0.1";
    first.tools.length = 0;
    const second = templateDraft("basic-trading", model);
    expect(second.tools.length).toBeGreaterThan(0);
    expect(second.workflow).toMatchObject({ instruments: [], baseSizeByInstrument: {} });
    expect(workflowDraft("trading")).toEqual(second.workflow);
    expect(() => subagentInput(second)).toThrow("instrument");
  });
  it("has only real permitted tools, no recursive delegation, no credentials or funded/automatic defaults", () => {
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
      if (draft.workflow?.purpose === "trading") { expect(draft.workflow.automaticEntriesEnabled).toBe(false); expect(draft.workflow.baseSizeByInstrument).toEqual({}); expect(draft.workflow.maxEntryNotional).toBe(""); }
      const workflow = draft.workflow;
      if (!workflow) throw new Error("Missing workflow");
      const configured = workflow.purpose === "analysis" ? { ...workflow, instruments: ["BTC"] } : workflow.purpose === "trading" ? { ...workflow, instruments: ["BTC"], baseSizeByInstrument: { BTC: "0.001" }, maxEntryNotional: "100" } : workflow;
      expect(subagentInput({ ...draft, workflow: configured }).tools).toEqual(template.tools);
    }
    expect(subagentTemplates.find((template) => template.purpose === "analysis")!.tools).not.toContain("submit_orders");
    expect(subagentTemplates.find((template) => template.purpose === "trading")!.tools).not.toContain("append_learning");
    expect(subagentTemplates.find((template) => template.purpose === "review")!.tools).not.toContain("submit_orders");
  });
  it("requires operator configuration instead of silently inventing instruments or funded sizing", () => {
    const model = { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" };
    expect(() => subagentInput(templateDraft("basic-trading", model))).toThrow();
    expect(() => subagentInput(templateDraft("basic-analysis", model))).toThrow();
    expect(() => templateDraft("remote-catalog:arbitrary", model)).toThrow("Unknown bundled");
    expect(templateDraft("basic-review", model).workflow).toMatchObject({ lookbackHours: 24, learningTargetSubagentIds: [] });
  });
});

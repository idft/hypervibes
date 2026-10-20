import { describe, expect, it } from "vitest";
import { accountToolNames, researchToolNames, subagentInput, taToolNames, workerToolNames } from "./subagent-input.js";
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
    { ...analysisManifest, defaults: { ...analysisManifest.defaults, completion: "wake" } },
    { ...analysisManifest, defaults: { ...analysisManifest.defaults, completion: "unknown" } },
    { ...analysisManifest, unknownExtraField: true },
    { ...tradingManifest, defaults: { ...tradingManifest.defaults, persistent: true } },
  ])("rejects unsupported/unsafe manifests %#", (manifest) => {
    expect(() => parseTemplateManifest(manifest)).toThrow();
  });
  it("formats analyst instructions with Markdown headings and blank lines between sections", () => {
    const template = parseTemplateManifest(analysisManifest);
    expect(template.instructions).toBe(analysisManifest.instructions.join("\n\n"));
    expect(template.instructions.startsWith("## Role\n\n")).toBe(true);
    expect(template.instructions).toContain("\n\n## Example strategy\n\n");
    expect(template.instructions.match(/^## /gm)).toHaveLength(6);
  });
  it.each(["save", "notify"])("preserves %s completion without waking the orchestrator", (completion) => {
    expect(parseTemplateManifest({ ...analysisManifest, defaults: { ...analysisManifest.defaults, completion } }).defaults.completion).toBe(completion);
  });
  it("defaults the analyst to a paused 15m schedule with a one-second delay and save-only completion", () => {
    const draft = templateDraft("basic-analysis", { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" });
    expect(draft).toMatchObject({ enabled: false, schedule: "15m", delaySeconds: 1, completion: "save" });
    expect(draft.instructions).toContain("use 15m candles");
  });
  it.each(subagentTemplates)("inherits only model fields from a full parent record for $id", (template) => {
    const model = { provider: "parent-provider", modelId: "parent-model", thinkingLevel: "high" };
    const parent = { ...templateDraft(template.id, model), id: "main:parent", schedule: "", delaySeconds: 30, completion: "wake", enabled: true, tools: ["delegate_subagent"] };
    expect(templateDraft(template.id, parent)).toEqual(templateDraft(template.id, model));
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
  it("gives the analyst Code Mode, all TA and artifact tools without trading permissions", () => {
    const draft = templateDraft("basic-analysis", { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" });
    expect(draft.tools).toEqual(expect.arrayContaining(["codemode", "research_candles", ...taToolNames, ...researchToolNames.filter((name) => name.startsWith("artifact_")), "publish_research"]));
    expect(draft.tools.some((name) => (accountToolNames as readonly string[]).includes(name))).toBe(false);
    expect(draft.instructions).toContain("evidenceAsOf");
    expect(draft.instructions).toContain("closed candles");
    expect(draft.instructions).toContain("not trading authority");
  });
  it("has only real permitted tools, no recursive delegation, no credentials or enabled/funded defaults", () => {
    const tools = [...createMemoryExtension("agent", {} as ExecutionContextService, {} as SharedDatabase).tools!, ...createHyperliquidExtension({} as HyperliquidService, "owner", "agent", {} as OrderService).tools!, ...createTaExtension({} as HyperliquidService["charts"], (() => { throw new Error("No execution in manifest test"); }) as ResearchResponder).tools!].map((tool) => tool.name);
    // Research tools are defined by the existing research extension, validated by
    // its integration tests; no remote catalog or import-time host is needed.
    tools.push(...researchToolNames);
    for (const template of subagentTemplates) {
      const draft = templateDraft(template.id, { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" });
      expect(draft).toMatchObject({ enabled: false, persistent: false, completion: template.defaults.completion, provider: "operator-selected", modelId: "operator-model" });
      expect(template.tools.every((tool) => (workerToolNames as readonly string[]).includes(tool) && tools.includes(tool))).toBe(true);
      expect(template.tools).not.toContain("delegate_subagent"); expect(template.tools).not.toContain("cancel_all_orders");
      expect(template.instructions).not.toMatch(/\{\{|\$\{/);
      expect(template.instructions).toContain("not a publication or order receipt");
      expect(subagentInput(draft).tools).toEqual(template.tools);
    }
    expect(subagentTemplates.find((template) => template.id === "basic-analysis")!.tools).not.toContain("submit_orders");
    expect(subagentTemplates.find((template) => template.id === "basic-trading")!.tools).not.toContain("append_insight");
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
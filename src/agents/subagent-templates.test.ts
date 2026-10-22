import { describe, expect, it } from "vitest";
import { accountToolNames, mainToolNames, researchToolNames, subagentInput, indicatorReadToolNames, workerToolNames } from "./subagent-input.js";
import analysisManifest from "./templates/basic-analysis.json" with { type: "json" };
import tradingManifest from "./templates/basic-trading.json" with { type: "json" };
import { parseTemplateCatalog, parseTemplateManifest } from "./template-manifest.js";
import { subagentTemplates, templateDraft } from "./subagent-templates.js";
import { createSubagentExtension } from "./subagent-extension.js";
import { createNotificationExtension } from "../notifications/durable-extension.js";
import type { NotificationService } from "../notifications/notification-service.js";
import { createMemoryExtension } from "../memory/durable-extension.js";
import { createHyperliquidExtension } from "../hyperliquid/durable-extension.js";
import { createIndicatorExtension } from "../indicators/durable-extension.js";
import type { IndicatorService } from "../indicators/indicator-service.js";
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
    { ...analysisManifest, tools: ["get_unknown_run"] },
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
  it("defaults the trader to save-only completion and account event subscriptions while remaining paused and unlinked to other workers", () => {
    const draft = templateDraft("basic-trading", { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" });
    expect(draft).toMatchObject({ enabled: false, completion: "save", schedule: "", triggers: { inboxEnabled: true, fills: true, orderChanges: true, researchSources: [], workerDelegates: [] } });
    expect(subagentInput(draft).triggers).toEqual(draft.triggers);
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
    expect(draft.tools).toEqual(expect.arrayContaining(["list_instruments", "codemode", "research_candles", ...indicatorReadToolNames, ...researchToolNames.filter((name) => name.startsWith("artifact_")), "publish_research"]));
    expect(draft.tools.some((name) => (accountToolNames as readonly string[]).includes(name))).toBe(false);
    expect(draft.instructions).toContain("evidenceAsOf");
    expect(draft.instructions).toContain("closed candles");
    expect(draft.instructions).toContain("not trading authority");
  });
  it("requires the analyst to verify quantitative and temporal claims before publishing", () => {
    const draft = templateDraft("basic-analysis", { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" });
    expect(draft.instructions).toContain("verify quantitative and temporal claims in Code Mode using returned data, not manually recopied values");
    expect(draft.instructions).toContain("Compute derived values over explicitly identified windows");
    expect(draft.instructions).toContain("rising lows, contracting bands or weaker volume need matching comparisons");
    expect(draft.instructions).toContain("a predicted indicator crossover needs its update calculation, not a comparison of the current gap with ATR");
    expect(draft.instructions).toContain("Distinguish computed observations from interpretations");
    expect(draft.instructions).toContain("If a claim cannot be verified, qualify or omit it");
  });
  it("publishes analyst reports as memories without redundant artifacts by default", () => {
    const draft = templateDraft("basic-analysis", { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" });
    expect(draft.instructions).toContain("Publish the analysis directly in the research memory by default; no artifact is required");
    expect(draft.instructions).toContain("Do not create an artifact that duplicates the memory report");
    expect(draft.instructions).toContain("Create artifacts only for distinct supporting material");
    expect(draft.instructions).toContain("Keep the memory understandable without opening attachments");
    expect(draft.instructions).toContain("Include artifactIds only when you created or used distinct supporting artifacts; otherwise omit them");
    expect(draft.instructions).not.toContain("Save useful supporting reports or datasets as artifacts");
  });
  it("teaches prose handoffs and trader interpretation rather than fixed signal labels", () => {
    const model = { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" };
    const analyst = templateDraft("basic-analysis", model), trader = templateDraft("basic-trading", model);
    expect(analyst.instructions).toContain("brief summary (maximum 500 characters)");
    expect(analyst.instructions).toContain("standalone content report");
    expect(analyst.instructions).toContain("no fixed headings, bias, setup labels or structured payload are required");
    expect(trader.instructions).toContain("use get_memory to read the full reports");
    expect(trader.instructions).toContain("fresh only describes evidence age");
  });
  it("teaches timeframe defaults, explicit analyst deadlines and separate correction/supersession semantics", () => {
    const model = { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" };
    const analyst = templateDraft("basic-analysis", model), trader = templateDraft("basic-trading", model);
    expect(analyst.instructions).toContain("primary decision timeframe, not the worker schedule");
    expect(analyst.instructions).toContain("two candle intervals (15m research: 30 minutes; 1d research: two days)");
    expect(analyst.instructions).toContain("Explicitly request a shorter or longer deadline");
    expect(analyst.instructions).toContain("up to seven days from evidence");
    expect(analyst.instructions).toContain("Use corrects only for complete replacement reports with the same evidence boundary");
    expect(analyst.instructions).toContain("Use supersedes for a complete replacement assessment based on strictly newer evidence");
    expect(analyst.instructions).toContain("even if the previous report expired");
    expect(trader.instructions).toContain("consumerStatus/consumerValidUntil");
    expect(trader.instructions).toContain("Check correctedBy and supersededBy even on fresh reports");
    expect(trader.instructions).toContain("never treat a replaced report as current merely because it is recent");
  });
  it("enables instrument discovery for every worker template and new orchestrators", () => {
    const model = { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" };
    expect(mainToolNames).toContain("list_instruments");
    for (const template of subagentTemplates) {
      expect(templateDraft(template.id, model).tools).toContain("list_instruments");
    }
  });
  it("gives the trader live market snapshots without treating them as candle confirmation", () => {
    const draft = templateDraft("basic-trading", { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" });
    expect(draft.tools).toContain("get_market_snapshot");
    expect(draft.instructions).toContain("refresh current prices and market context before evaluating a new entry or price-dependent position management");
    expect(draft.instructions).toContain("Distinguish its mid, mark and oracle prices from executable quotes");
    expect(draft.instructions).toContain("A snapshot cannot confirm a closed-candle breakout or volume condition");
    expect(draft.instructions).toContain("report that uncertainty rather than claiming it occurred");
    expect(draft.instructions).toContain("Do not poll or wait for a price trigger");
  });
  it("enables insight tools only for the reviewer by default", () => {
    const insightTools = ["get_insights", "append_insight", "replace_insights"];
    for (const template of subagentTemplates) {
      const draft = templateDraft(template.id, { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" });
      expect(draft.tools.filter((tool) => insightTools.includes(tool))).toEqual(template.id === "basic-review" ? insightTools : []);
    }
    expect(mainToolNames.filter((tool) => insightTools.includes(tool))).toEqual([]);
  });
  it("has only real permitted tools, no recursive delegation, no credentials or enabled/funded defaults", () => {
    const tools = [...createNotificationExtension("agent", {} as NotificationService, {} as ExecutionContextService).tools!, ...createMemoryExtension("agent", {} as ExecutionContextService, {} as SharedDatabase).tools!, ...createHyperliquidExtension({} as HyperliquidService, "owner", "agent", {} as OrderService).tools!, ...createIndicatorExtension("agent", {} as IndicatorService, {} as ExecutionContextService, (() => { throw new Error("No execution in manifest test"); }) as ResearchResponder).tools!].map((tool) => tool.name);
    // Research tools are defined by the existing research extension, validated by
    // its integration tests; no remote catalog or import-time host is needed.
    tools.push("list_instruments", ...researchToolNames, ...createSubagentExtension({} as Parameters<typeof createSubagentExtension>[0], "agent", { name: "test-hyperliquid" }, [], {} as ExecutionContextService).extension.tools!.map((tool) => tool.name));
    for (const template of subagentTemplates) {
      const draft = templateDraft(template.id, { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" });
      expect(draft).toMatchObject({ enabled: false, persistent: false, completion: template.defaults.completion, provider: "operator-selected", modelId: "operator-model" });
      expect(template.tools.every((tool) => (workerToolNames as readonly string[]).includes(tool) && tools.includes(tool))).toBe(true);
      expect(template.tools).not.toContain("delegate_subagent"); expect(template.tools).not.toContain("cancel_all_orders");
      expect(template.instructions).not.toMatch(/\{\{|\$\{/);
      expect(template.instructions).toMatch(/not a publication or order receipt|not that its work succeeded or a review\/research report was published/);
      expect(subagentInput(draft).tools).toEqual(template.tools);
    }
    expect(subagentTemplates.find((template) => template.id === "basic-analysis")!.tools).not.toContain("submit_orders");
    expect(subagentTemplates.find((template) => template.id === "basic-trading")!.tools).not.toContain("append_insight");
    expect(subagentTemplates.find((template) => template.id === "basic-review")!.tools).not.toContain("submit_orders");
  });
  it("gives Review complete read-only evidence access and automatic scoped insight curation", () => {
    const draft = templateDraft("basic-review", { provider: "operator-selected", modelId: "operator-model", thinkingLevel: "" });
    expect(draft.tools).toEqual(expect.arrayContaining(["codemode", "artifact_read", "research_execution_audit", "list_subagent_runs", "get_subagent_run", "research_journal", "append_insight", "replace_insights"]));
    expect(draft.tools).not.toEqual(expect.arrayContaining(["submit_orders"]));
    expect(draft.tools.some((tool) => ["submit_orders", "cancel_orders", "cancel_all_orders", "delegate_subagent", "artifact_delete", "artifact_retention"].includes(tool))).toBe(false);
    expect(draft.instructions).toContain("host admittedAtMs");
    expect(draft.instructions).toContain("do not invent independent worker portfolios");
    expect(draft.instructions).toContain("automatically curate");
    expect(draft.instructions).toContain("No insight change is a valid successful outcome");
    expect(draft.instructions).toContain("without duplicating an already-published review");
    expect(draft.instructions).toContain("24 hours");
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
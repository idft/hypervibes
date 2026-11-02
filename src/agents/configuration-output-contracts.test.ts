import { Check } from "typebox/value";
import { describe, expect, it } from "vitest";
import { ArchiveWorkerResultSchema, CreateWorkerResultSchema, GetConfigurationResultSchema, ListAgentToolsResultSchema, ListWorkerTemplatesResultSchema, UpdateConfigurationResultSchema } from "./configuration-output-contracts.js";
import { subagentTemplate, templateDraft } from "./subagent-templates.js";

const definition = {
  ...templateDraft("basic-analysis", { provider: "faux", modelId: "faux-1", thinkingLevel: "" }),
  id: "worker:test", agentId: "agent", kind: "worker", archived: false,
  conversationId: null, nextRunAt: null, templateOrigin: null, createdAt: 1, updatedAt: 1,
};
const notifications = { enabled: true, submissions: false, fills: true };

describe("configuration output contracts", () => {
  it("validates full definitions and instruction pagination", () => {
    const value = { definition, instructionsPage: { offset: 0, total: definition.instructions.length, nextOffset: null } };
    expect(Check(GetConfigurationResultSchema, value)).toBe(true);
    expect(Check(GetConfigurationResultSchema, { ...value, definition: { ...definition, conversationId: "1" } })).toBe(false);
    expect(Check(GetConfigurationResultSchema, { ...value, definition: { ...definition, credentials: "secret" } })).toBe(false);
    expect(Check(CreateWorkerResultSchema, { definition, effect: "Created paused" })).toBe(true);
  });
  it("validates overview statuses, exact risk strings and available models", () => {
    const header = { id: "main:agent", name: "Orchestrator", description: "", enabled: true, archived: false, tools: [], provider: "faux", modelId: "faux-1", thinkingLevel: "", schedule: "", nextRunAt: null, updatedAt: 1 };
    const page = { offset: 0, total: 0, nextOffset: null };
    const value = {
      agent: { id: "agent", name: "Desk", enabled: true, state: "ready" }, notifications,
      risk: { tradingEnabled: false, allowedInstruments: ["BTC"], maxOrderNotional: "100.25", maxTotalNotional: null, readOnly: true },
      telegram: { configured: true, linked: true, botUsername: "desk_bot", error: "Check settings" },
      orchestrator: header, workers: [], workersPage: page,
      models: [{ provider: "faux", modelId: "faux-1", name: "Faux", providerName: "Faux", thinkingLevels: ["off"], contextWindow: 100000, inputCost: 0, outputCost: 0, reasoning: false }],
      modelsPage: { ...page, total: 1 },
    };
    expect(Check(GetConfigurationResultSchema, value)).toBe(true);
    expect(Check(GetConfigurationResultSchema, { ...value, risk: { ...value.risk, maxOrderNotional: 100.25 } })).toBe(false);
    expect(Check(GetConfigurationResultSchema, { ...value, telegram: { ...value.telegram, token: "secret" } })).toBe(false);
  });
  it("requires tool discovery metadata and typed pagination", () => {
    const value = { subagentId: "main:agent", updatedAt: 1, offset: 0, total: 1, nextOffset: null, tools: [{ name: "send_notification", description: "Send an update", group: "notifications", enabled: true, mandatory: false, exposedNow: true }] };
    expect(Check(ListAgentToolsResultSchema, value)).toBe(true);
    expect(Check(ListAgentToolsResultSchema, { ...value, nextOffset: "next" })).toBe(false);
    expect(Check(ListAgentToolsResultSchema, { ...value, tools: [{ name: "send_notification" }] })).toBe(false);
  });
  it("distinguishes template headers from complete template details", () => {
    const template = subagentTemplate("basic-review");
    const { id, version, name, description, tools, defaults } = template;
    expect(Check(ListWorkerTemplatesResultSchema, { templates: [{ id, version, name, description, tools, defaults }] })).toBe(true);
    expect(Check(ListWorkerTemplatesResultSchema, { template })).toBe(true);
    expect(Check(ListWorkerTemplatesResultSchema, { template: { ...template, defaults: { ...defaults, enabled: true } } })).toBe(false);
  });
  it("validates each mutation section rather than accepting arbitrary objects", () => {
    const value = { subagentId: "worker:test", section: "tools", before: { tools: [] }, after: { tools: ["send_notification"] }, updatedAt: 2, effect: "Future assignments", executionToolsAdded: [] };
    expect(Check(UpdateConfigurationResultSchema, value)).toBe(true);
    expect(Check(UpdateConfigurationResultSchema, { ...value, after: { instructions: "Wrong section" } })).toBe(false);
    expect(Check(UpdateConfigurationResultSchema, { ...value, section: "risk" })).toBe(false);
    expect(Check(UpdateConfigurationResultSchema, { section: "notifications", before: notifications, after: { ...notifications, fills: false }, effect: "Immediate" })).toBe(true);
    expect(Check(UpdateConfigurationResultSchema, { section: "notifications", before: notifications, after: {}, effect: "Immediate" })).toBe(false);
  });
  it("requires archive responses to report an archived, disabled worker", () => {
    const value = { subagentId: "worker:test", before: { archived: false, enabled: true }, after: { archived: true, enabled: false }, effect: "Archived, not deleted" };
    expect(Check(ArchiveWorkerResultSchema, value)).toBe(true);
    expect(Check(ArchiveWorkerResultSchema, { ...value, after: { archived: true, enabled: true } })).toBe(false);
  });
});

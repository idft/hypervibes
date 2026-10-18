import { AgentError } from "./agent-input.js";
import { instrument, interval, topic, type ResearchRequirement } from "../memory/memory-contracts.js";
import { array, fields, integer, object, positiveDecimal, text, unique } from "../memory/validation.js";

export type WorkerWorkflow =
  | { version: 1; purpose: "analysis"; instruments: string[]; interval: string; topic: string; maxResearchAgeSeconds: number; strategy: string }
  | { version: 1; purpose: "trading"; instruments: string[]; automaticEntriesEnabled: boolean; requiredResearch: ResearchRequirement[]; entryStrategy: string; exitStrategy: string; baseSizeByInstrument: Record<string, string>; maxEntryNotional: string; takeProfitBps: number; stopLossBps: number }
  | { version: 1; purpose: "review"; lookbackHours: number; learningTargetSubagentIds: string[] };
const workerId = (value: unknown) => text(value, 100, "worker ID");
export function researchRequirements(value: unknown): ResearchRequirement[] {
  const result = array(value, 20, (item) => {
    const row = fields(item, ["subagentId", "topic", "interval", "maxAgeSeconds"]);
    return { subagentId: workerId(row.subagentId), topic: topic(row.topic), interval: interval(row.interval), maxAgeSeconds: integer(row.maxAgeSeconds, 1, 7 * 86400, "research maximum age") };
  });
  unique(result.map((r) => `${r.subagentId}:${r.topic}:${r.interval}`));
  return result;
}
/** Catalog and same-Agent target validation is additionally required by the host. */
export function workerWorkflow(value: unknown): WorkerWorkflow { return parseWorkflow(value, false); }
/** Template drafts may omit instruments/sizing, but can never enable automatic entry. */
export function templateWorkflow(value: unknown): WorkerWorkflow { return parseWorkflow(value, true); }
function parseWorkflow(value: unknown, draft: boolean): WorkerWorkflow {
  const input = object(value);
  integer(input.version, 1, 1, "workflow version");
  if (input.purpose === "review") {
    const row = fields(input, ["version", "purpose", "lookbackHours", "learningTargetSubagentIds"]);
    return { version: 1, purpose: "review", lookbackHours: integer(row.lookbackHours, 1, 90 * 24, "lookback hours"), learningTargetSubagentIds: unique(array(row.learningTargetSubagentIds, 20, workerId)) };
  }
  const instruments = unique(array(input.instruments, 10, instrument));
  if (!draft && !instruments.length) throw new AgentError("Select at least one instrument");
  if (input.purpose === "analysis") {
    const row = fields(input, ["version", "purpose", "instruments", "interval", "topic", "maxResearchAgeSeconds", "strategy"]);
    return { version: 1, purpose: "analysis", instruments, interval: interval(row.interval), topic: topic(row.topic), maxResearchAgeSeconds: integer(row.maxResearchAgeSeconds, 1, 7 * 86400, "maximum research age"), strategy: text(row.strategy, 8000, "analysis strategy") };
  }
  if (input.purpose === "trading") {
    const row = fields(input, ["version", "purpose", "instruments", "automaticEntriesEnabled", "requiredResearch", "entryStrategy", "exitStrategy", "baseSizeByInstrument", "maxEntryNotional", "takeProfitBps", "stopLossBps"]);
    if (row.automaticEntriesEnabled !== undefined && typeof row.automaticEntriesEnabled !== "boolean") throw new AgentError("Invalid automatic-entry enablement");
    if (draft && row.automaticEntriesEnabled === true) throw new AgentError("Template automatic entries must start disabled");
    const sizes = object(row.baseSizeByInstrument);
    if (Object.keys(sizes).length !== instruments.length || Object.keys(sizes).some((coin) => !instruments.includes(coin))) throw new AgentError("Configure one exact base size per selected instrument");
    const result: WorkerWorkflow = { version: 1, purpose: "trading", instruments, automaticEntriesEnabled: row.automaticEntriesEnabled === true, requiredResearch: researchRequirements(row.requiredResearch), entryStrategy: text(row.entryStrategy, 8000, "entry strategy"), exitStrategy: text(row.exitStrategy, 8000, "exit strategy"), baseSizeByInstrument: Object.fromEntries(instruments.map((coin) => [coin, positiveDecimal(sizes[coin])])), maxEntryNotional: draft && row.maxEntryNotional === "" ? "" : positiveDecimal(row.maxEntryNotional), takeProfitBps: integer(row.takeProfitBps, 1, 9999, "TP basis points"), stopLossBps: integer(row.stopLossBps, 1, 9999, "SL basis points") };
    if (result.automaticEntriesEnabled && !result.requiredResearch.length) throw new AgentError("Automatic entries require configured research producers");
    return result;
  }
  throw new AgentError("Unsupported workflow purpose");
}
export function workflowReadiness(workflow: WorkerWorkflow, workers: readonly { id: string; enabled: boolean; archived: boolean }[]): string[] {
  const targets = workflow.purpose === "trading" ? workflow.requiredResearch.map((r) => r.subagentId) : workflow.purpose === "review" ? workflow.learningTargetSubagentIds : [];
  return [...new Set(targets)].flatMap((id) => {
    const worker = workers.find((candidate) => candidate.id === id);
    return !worker ? [`Worker ${id} is not in this Agent.`] : worker.archived || !worker.enabled ? [`Worker ${id} is disabled or archived.`] : [];
  });
}

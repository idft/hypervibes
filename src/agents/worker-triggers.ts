import { AgentError } from "./agent-input.js";

export interface WorkerTriggers {
  inboxEnabled: boolean;
  workerDelegates: string[];
  researchSources: string[];
  fills: boolean;
  orderChanges: boolean;
}
export const defaultWorkerTriggers: WorkerTriggers = { inboxEnabled: false, workerDelegates: [], researchSources: [], fills: false, orderChanges: false };
export function workerTriggers(value: unknown): WorkerTriggers {
  if (value === undefined) return { ...defaultWorkerTriggers, workerDelegates: [], researchSources: [] };
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new AgentError("Invalid event triggers");
  const row = value as Record<string, unknown>;
  if (Object.keys(row).some((key) => !Object.hasOwn(defaultWorkerTriggers, key))) throw new AgentError("Unknown event trigger setting");
  const bool = (key: string) => { if (typeof row[key] !== "boolean") throw new AgentError(`Invalid ${key}`); return row[key]; };
  const ids = (key: string): string[] => {
    const values = row[key];
    if (!Array.isArray(values) || values.length > 50 || values.some((id: unknown) => typeof id !== "string" || !/^worker:[a-zA-Z0-9-]{1,100}$/.test(id))) throw new AgentError(`Invalid ${key}`);
    return [...new Set(values as string[])];
  };
  const result = { inboxEnabled: bool("inboxEnabled"), workerDelegates: ids("workerDelegates"), researchSources: ids("researchSources"), fills: bool("fills"), orderChanges: bool("orderChanges") };
  if (!result.inboxEnabled && (result.workerDelegates.length || result.researchSources.length || result.fills || result.orderChanges)) throw new AgentError("Enable the worker inbox before configuring event triggers");
  return result;
}

export type WorkerIntent = "operator_trade" | "evaluation" | "account_event";
export interface WorkerRequestPayload {
  intent: WorkerIntent;
  reason: "delegation" | "research" | "fill" | "order_status";
  task: string;
  callerSubagentId: string | null;
  callerRunId: string | null;
  researchRefs: string[];
}

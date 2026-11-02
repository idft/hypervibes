import { Type } from "typebox";

const strict = { additionalProperties: false };
const string = Type.String();
const boolean = Type.Boolean();
const timestamp = Type.Integer({ minimum: 0 });
const nullableTimestamp = Type.Union([timestamp, Type.Null()]);
const nullableString = Type.Union([string, Type.Null()]);
const tools = Type.Array(string);
const page = Type.Object({ offset: timestamp, total: timestamp, nextOffset: nullableTimestamp }, strict);
const notifications = Type.Object({ enabled: boolean, submissions: boolean, fills: boolean }, strict);
const triggers = Type.Object({ inboxEnabled: boolean, workerDelegates: tools, researchSources: tools, fills: boolean, orderChanges: boolean }, strict);
const completion = Type.Union([Type.Literal("save"), Type.Literal("notify"), Type.Literal("wake")]);
const headerProperties = {
  id: string, name: string, description: string, enabled: boolean, archived: boolean,
  tools, provider: string, modelId: string, thinkingLevel: string,
  schedule: string, nextRunAt: nullableTimestamp, updatedAt: timestamp,
};
const header = Type.Object(headerProperties, strict);
const definition = Type.Object({
  ...headerProperties,
  agentId: string,
  kind: Type.Union([Type.Literal("orchestrator"), Type.Literal("worker")]),
  instructions: string,
  triggers,
  templateOrigin: Type.Union([Type.Object({ id: string, version: Type.Integer(), hash: string }, strict), Type.Null()]),
  delegatable: boolean, persistent: boolean, conversationId: nullableTimestamp,
  delaySeconds: Type.Integer(), completion, timeoutSeconds: Type.Integer(), createdAt: timestamp,
}, strict);

export const GetConfigurationResultSchema = Type.Union([
  Type.Object({
    definition,
    instructionsPage: Type.Object({ offset: timestamp, total: timestamp, nextOffset: nullableTimestamp }, strict),
  }, strict),
  Type.Object({
    agent: Type.Object({ id: string, name: string, enabled: boolean, state: string }, strict),
    notifications,
    risk: Type.Object({ tradingEnabled: boolean, allowedInstruments: Type.Union([tools, Type.Null()]), maxOrderNotional: nullableString, maxTotalNotional: nullableString, readOnly: Type.Literal(true) }, strict),
    telegram: Type.Object({ configured: boolean, linked: boolean, botUsername: nullableString, error: Type.Optional(string) }, strict),
    orchestrator: header,
    workers: Type.Array(header), workersPage: page,
    models: Type.Array(Type.Object({ provider: string, modelId: string, name: string, providerName: string, thinkingLevels: tools, contextWindow: Type.Number(), inputCost: Type.Number(), outputCost: Type.Number(), reasoning: boolean }, strict)),
    modelsPage: page,
  }, strict),
]);

export const ListAgentToolsResultSchema = Type.Object({
  subagentId: string, updatedAt: timestamp,
  ...page.properties,
  tools: Type.Array(Type.Object({ name: string, description: string, group: string, enabled: boolean, mandatory: boolean, exposedNow: Type.Optional(boolean) }, strict)),
}, strict);

const templateHeaderProperties = {
  id: string, version: Type.Integer(), name: string, description: string, tools,
  defaults: Type.Object({ persistent: Type.Literal(false), enabled: Type.Literal(false), delegatable: boolean, schedule: string, delaySeconds: Type.Integer(), completion: Type.Union([Type.Literal("save"), Type.Literal("notify")]), timeoutSeconds: Type.Integer(), triggers: Type.Optional(triggers) }, strict),
};
export const ListWorkerTemplatesResultSchema = Type.Union([
  Type.Object({ templates: Type.Array(Type.Object(templateHeaderProperties, strict)) }, strict),
  Type.Object({ template: Type.Object({ ...templateHeaderProperties, formatVersion: Type.Literal(1), instructions: string }, strict) }, strict),
]);

const sectionResults = [
  { section: "tools", value: Type.Object({ tools }, strict) },
  { section: "identity", value: Type.Object({ name: string, description: string }, strict) },
  { section: "instructions", value: Type.Object({ instructions: string }, strict) },
  { section: "model", value: Type.Object({ provider: string, modelId: string, thinkingLevel: string }, strict) },
  { section: "assignments", value: Type.Object({ delegatable: boolean, persistent: boolean, timeoutSeconds: Type.Integer() }, strict) },
  { section: "enabled", value: Type.Object({ enabled: boolean, nextRunAt: nullableTimestamp }, strict) },
  { section: "schedule", value: Type.Object({ schedule: string, delaySeconds: Type.Integer(), completion, nextRunAt: nullableTimestamp }, strict) },
  { section: "triggers", value: Type.Object({ triggers }, strict) },
].map(({ section, value }) => Type.Object({
  subagentId: string, section: Type.Literal(section), before: value, after: value,
  updatedAt: timestamp, effect: string, executionToolsAdded: tools,
}, strict));

export const UpdateConfigurationResultSchema = Type.Union([
  ...sectionResults,
  Type.Object({ section: Type.Literal("notifications"), before: notifications, after: notifications, effect: string }, strict),
]);
export const CreateWorkerResultSchema = Type.Object({ definition, effect: string }, strict);
export const ArchiveWorkerResultSchema = Type.Object({
  subagentId: string,
  before: Type.Object({ archived: boolean, enabled: boolean }, strict),
  after: Type.Object({ archived: Type.Literal(true), enabled: Type.Literal(false) }, strict),
  effect: string,
}, strict);

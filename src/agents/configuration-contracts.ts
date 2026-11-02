import { Type, type Static } from "typebox";

export const configurationToolNames = ["get_agent_configuration", "list_agent_tools", "list_worker_templates", "update_agent_configuration", "create_worker", "archive_worker"] as const;
export type ConfigurationToolName = (typeof configurationToolNames)[number];
const strict = { additionalProperties: false };
const id = Type.String({ minLength: 1, maxLength: 200 });
const names = Type.Array(Type.String({ minLength: 1, maxLength: 100 }), { maxItems: 150, uniqueItems: true });
const text = (maximum: number) => Type.String({ maxLength: maximum });
const triggers = Type.Object({
  inboxEnabled: Type.Optional(Type.Boolean()),
  researchSources: Type.Optional(Type.Array(id, { maxItems: 50, uniqueItems: true })),
  workerDelegates: Type.Optional(Type.Array(id, { maxItems: 50, uniqueItems: true })),
  fills: Type.Optional(Type.Boolean()),
  orderChanges: Type.Optional(Type.Boolean()),
}, strict);
export const ConfigurationChangeSchema = Type.Union([
  Type.Object({ section: Type.Literal("tools"), add: Type.Optional(names), remove: Type.Optional(names) }, strict),
  Type.Object({ section: Type.Literal("identity"), name: Type.Optional(text(80)), description: Type.Optional(text(500)) }, strict),
  Type.Object({ section: Type.Literal("instructions"), instructions: Type.String({ minLength: 1, maxLength: 32000 }) }, strict),
  Type.Object({ section: Type.Literal("model"), provider: Type.Optional(text(200)), modelId: Type.Optional(text(300)), thinkingLevel: Type.Optional(text(32)) }, strict),
  Type.Object({ section: Type.Literal("assignments"), delegatable: Type.Optional(Type.Boolean()), persistent: Type.Optional(Type.Boolean()), timeoutSeconds: Type.Optional(Type.Integer({ minimum: 30, maximum: 3600 })) }, strict),
  Type.Object({ section: Type.Literal("enabled"), enabled: Type.Boolean() }, strict),
  Type.Object({ section: Type.Literal("schedule"), schedule: Type.Optional(text(8)), delaySeconds: Type.Optional(Type.Integer({ minimum: 0, maximum: 300 })), completion: Type.Optional(Type.Union([Type.Literal("save"), Type.Literal("notify"), Type.Literal("wake")])) }, strict),
  Type.Object({ section: Type.Literal("triggers"), triggers }, strict),
  Type.Object({ section: Type.Literal("notifications"), enabled: Type.Optional(Type.Boolean()), submissions: Type.Optional(Type.Boolean()), fills: Type.Optional(Type.Boolean()) }, strict),
]);
export const UpdateConfigurationSchema = Type.Object({
  subagent_id: Type.Optional(id),
  expected_updated_at: Type.Optional(Type.Integer({ minimum: 0, description: "Required for worker/orchestrator edits; use updatedAt from get_agent_configuration. Not used for notification preferences." })),
  change: ConfigurationChangeSchema,
}, strict);
export type ConfigurationChange = Static<typeof ConfigurationChangeSchema>;
export type UpdateConfigurationInput = Static<typeof UpdateConfigurationSchema>;
export const CreateWorkerSchema = Type.Object({ template_id: id, name: Type.Optional(text(80)), description: Type.Optional(text(500)) }, strict);
export const ArchiveWorkerSchema = Type.Object({ subagent_id: id, expected_updated_at: Type.Integer({ minimum: 0 }) }, strict);
export const ListAgentToolsSchema = Type.Object({
  subagent_id: Type.Optional(id),
  query: Type.Optional(text(100)),
  offset: Type.Optional(Type.Integer({ minimum: 0 })),
  limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 20 })),
}, strict);
export const GetConfigurationSchema = Type.Object({
  subagent_id: Type.Optional(id),
  instructions_offset: Type.Optional(Type.Integer({ minimum: 0 })),
  instructions_limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 8000 })),
  models_offset: Type.Optional(Type.Integer({ minimum: 0 })),
  workers_offset: Type.Optional(Type.Integer({ minimum: 0 })),
}, strict);

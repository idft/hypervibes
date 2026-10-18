import { AgentError } from "./agent-input.js";
import { candleIntervals, workerToolNames } from "./subagent-input.js";
import { array, choice, fields, integer, text, unique } from "../memory/validation.js";

export interface SubagentTemplate {
  formatVersion: 1; id: string; version: number;
  name: string; description: string; instructions: string; tools: readonly string[];
  defaults: { persistent: false; enabled: false; delegatable: boolean; schedule: string; delaySeconds: number; completion: "notify"; timeoutSeconds: number };
}

/** Data only: neither parsing nor selecting a manifest grants tools or starts work. */
export function parseTemplateManifest(value: unknown): SubagentTemplate {
  const row = fields(value, ["formatVersion", "id", "version", "name", "description", "instructions", "tools", "defaults"]);
  integer(row.formatVersion, 1, 1, "template format version");
  const id = text(row.id, 100, "template ID");
  if (!/^[a-z][a-z0-9-]*$/.test(id)) throw new AgentError("Invalid template ID");
  const defaults = fields(row.defaults, ["persistent", "enabled", "delegatable", "schedule", "delaySeconds", "completion", "timeoutSeconds"]);
  if (defaults.enabled !== false || defaults.persistent !== false || defaults.completion !== "notify") throw new AgentError("Templates must start paused, with fresh conversations and notification-only completion");
  if (typeof defaults.delegatable !== "boolean") throw new AgentError("Invalid template delegation setting");
  const schedule = choice(defaults.schedule, Object.keys(candleIntervals));
  const delaySeconds = integer(defaults.delaySeconds, 0, 300, "template delay");
  if (delaySeconds >= candleIntervals[schedule as keyof typeof candleIntervals]) throw new AgentError("Delay must be shorter than the candle interval");
  const instructions = text(array(row.instructions, 100, (line) => text(line, 32000, "instruction paragraph")).join("\n"), 32000, "instructions");
  return {
    formatVersion: 1, id, version: integer(row.version, 1, Number.MAX_SAFE_INTEGER, "template version"),
    name: text(row.name, 80, "template name"), description: text(row.description, 500, "template description"), instructions,
    tools: unique(array(row.tools, workerToolNames.length, (tool) => choice(tool, workerToolNames))),
    defaults: { enabled: false, persistent: false, delegatable: defaults.delegatable, schedule, delaySeconds, completion: "notify", timeoutSeconds: integer(defaults.timeoutSeconds, 30, 3600, "template timeout") },
  };
}

export function parseTemplateCatalog(value: unknown): readonly SubagentTemplate[] {
  const templates = array(value, 100, parseTemplateManifest);
  unique(templates.map((template) => template.id));
  return templates;
}
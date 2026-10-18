import analysis from "./templates/basic-analysis.json" with { type: "json" };
import trading from "./templates/basic-trading.json" with { type: "json" };
import review from "./templates/basic-review.json" with { type: "json" };
import type { SubagentInput } from "./subagent-input.js";
import { AgentError } from "./agent-input.js";
import { parseTemplateCatalog } from "./template-manifest.js";
export type { SubagentTemplate } from "./template-manifest.js";

// Explicit bundled catalog: no filesystem scanning, network access, or executable
// template modules. The same data parser can validate future external catalogs.
export const subagentTemplates = parseTemplateCatalog([analysis, trading, review]);
export function subagentTemplate(id: string) {
  const template = subagentTemplates.find((entry) => entry.id === id);
  if (!template) throw new AgentError("Unknown bundled template");
  return template;
}
export function templateDraft(id: string, model: Pick<SubagentInput, "provider" | "modelId" | "thinkingLevel">): SubagentInput {
  const template = subagentTemplate(id);
  return { ...template.defaults, ...model, name: template.name, description: template.description, instructions: template.instructions, tools: [...template.tools] };
}
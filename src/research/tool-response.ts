import type { Context, JsonValue } from "@earendil-works/chord";
import type { ToolExecutionApi } from "@earendil-works/pi-durable";
import type { ArtifactService } from "./artifacts.js";

/** Normalize service interfaces to JSON and bound only the model-facing preview. */
export function jsonResult(value: object, truncate = true) {
  const text = JSON.stringify(value);
  const preview = truncate && text.length > 12_000 ? `${text.slice(0, 12_000)}\n[Model preview truncated; use Code Mode to process the structured result or request a smaller page.]` : text;
  return { output: [{ type: "text" as const, text: preview }], structuredOutput: JSON.parse(text) as JsonValue };
}

export function createResearchResponder(artifacts: ArtifactService, agentId: string, indicators = false) {
  const summary = (value: object) => Object.fromEntries(Object.entries(value).filter(([key]) => !["candles", "trades", "values", "outputs", "levels", "fills", "markers", "drawings"].includes(key)));
  return async (saveAs: string | undefined, api: ToolExecutionApi, context: Context, produce: () => Promise<object>, labels: { instrument?: string; interval?: string } = {}) => {
    if (saveAs) {
      if (!(await api.agent(context)).callable.some((tool) => tool.name === "artifact_create")) throw new Error("Saving exports also requires artifact_create permission");
      const existing = await artifacts.operation(agentId, `export:${api.taskId}`);
      if (existing) {
        const saved = await artifacts.get(agentId, existing.id);
        return jsonResult({ artifact: existing, reference: `artifact:${existing.id}`, summary: summary(JSON.parse(saved.payload.toString("utf8")) as object) });
      }
    }
    const value = await produce();
    if (!saveAs) return jsonResult(value, !indicators);
    const artifact = await artifacts.create(agentId, `export:${api.taskId}`, { title: saveAs, mediaType: "application/json", content: JSON.stringify(value), ...labels }, { conversationId: api.conversationId, taskId: api.taskId });
    return jsonResult({ artifact, reference: `artifact:${artifact.id}`, summary: summary(value) });
  };
}
export type ResearchResponder = ReturnType<typeof createResearchResponder>;

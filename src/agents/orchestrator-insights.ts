import { defineDoc, defineExtension, section } from "@earendil-works/pi-durable";
import type { ExecutionContextService } from "./execution-context.js";

export const orchestratorInsightsSection = "agent_insights";

/** The first placed input identifies a response across generation handovers,
 * retries and recovery. Steering may add inputs, but does not replace the first. */
export const OrchestratorInsightsDoc = defineDoc<{ input: number | null; instructions: string }>({
  kind: "hv.orchestrator-insights", version: 1, scope: "conversation", history: "latest", fork: "initial",
  initial: () => ({ input: null, instructions: "" }),
});

/** Selected only by the root, regardless of its configured tools. */
export function createOrchestratorInsightsExtension(agentId: string, execution: ExecutionContextService) {
  return defineExtension({ name: "orchestrator-insights", sections: [
    section(orchestratorInsightsSection, (input, context) => execution.orchestratorInsights(agentId, input.conversationId, context)),
  ] });
}

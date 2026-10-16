import { createFileRoute } from "@tanstack/react-router";
import { SubagentRunPage } from "../components/subagent-run-page.js";
import { getSubagentRun } from "../web/subagents.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/sub-agents/$subagentId_/runs/$runId")({
  loader: ({ params }) => getSubagentRun({ data: { agentId: params.agentId, id: params.subagentId, runId: params.runId } }),
  component: Page,
});
function Page() {
  const result = Route.useLoaderData();
  return result.ok ? <SubagentRunPage key={result.data.run.id} initial={result.data} /> : <p role="alert" className="text-red-300">{result.error}</p>;
}

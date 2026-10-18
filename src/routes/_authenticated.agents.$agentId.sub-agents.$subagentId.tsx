import { createFileRoute } from "@tanstack/react-router";
import { SubagentDetailPage } from "../components/subagent-detail-page.js";
import { getSubagent } from "../web/subagents.js";
import { getModelChoices } from "../web/providers.js";
export const Route = createFileRoute("/_authenticated/agents/$agentId/sub-agents/$subagentId")({
  loader: async ({ params }) => ({ result: await getSubagent({ data: { agentId: params.agentId, id: params.subagentId } }), choices: await getModelChoices() }), component: Page,
});
function Page() {
  const { result, choices } = Route.useLoaderData(), { agentId } = Route.useParams();
  return result.ok ? <SubagentDetailPage key={result.data.definition.id} agentId={agentId} initial={result.data} choices={choices} /> : <p role="alert" className="text-red-300">{result.error}</p>;
}
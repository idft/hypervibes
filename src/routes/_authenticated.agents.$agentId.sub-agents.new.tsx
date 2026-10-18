import { createFileRoute } from "@tanstack/react-router";
import { SubagentDetailPage } from "../components/subagent-detail-page.js";
import { listSubagents } from "../web/subagents.js";
import { getModelChoices } from "../web/providers.js";
export const Route = createFileRoute("/_authenticated/agents/$agentId/sub-agents/new")({ loader: async ({ params }) => ({ choices: await getModelChoices(), workers: await listSubagents({ data: { agentId: params.agentId } }) }), component: Page });
function Page() {
  const { choices, workers } = Route.useLoaderData(), { agentId } = Route.useParams();
  return workers.ok ? <SubagentDetailPage agentId={agentId} choices={choices} workers={workers.data} /> : <p role="alert" className="text-red-300">{workers.error}</p>;
}

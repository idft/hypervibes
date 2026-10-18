import { createFileRoute } from "@tanstack/react-router";
import { SubagentDetailPage } from "../components/subagent-detail-page.js";
import { getModelChoices } from "../web/providers.js";
export const Route = createFileRoute("/_authenticated/agents/$agentId/sub-agents/new")({ loader: async () => ({ choices: await getModelChoices() }), component: Page });
function Page() {
  const { choices } = Route.useLoaderData(), { agentId } = Route.useParams();
  return <SubagentDetailPage agentId={agentId} choices={choices} />;
}
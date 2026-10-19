import { createFileRoute } from "@tanstack/react-router";
import { NewSubagentPage } from "../components/new-subagent-page.js";
import { listSubagents } from "../web/subagents.js";
import { getModelChoices } from "../web/providers.js";
export const Route = createFileRoute("/_authenticated/agents/$agentId/sub-agents/new")({ loader: async ({ params }) => { const [result, choices] = await Promise.all([listSubagents({ data: { agentId: params.agentId } }), getModelChoices()]); return { result, choices }; }, component: Page });
function Page() {
  const { result, choices } = Route.useLoaderData(), { agentId } = Route.useParams();
  if (!result.ok) return <p role="alert" className="text-red-300">{result.error}</p>;
  const main = result.data.find((entry) => entry.kind === "orchestrator");
  return main ? <NewSubagentPage agentId={agentId} model={main} choices={choices} /> : <p role="alert" className="text-red-300">Parent Agent model is unavailable. Open the Agent chat to initialize it, then retry.</p>;
}
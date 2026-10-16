import { createFileRoute } from "@tanstack/react-router";
import { SubagentsPage } from "../components/subagents-page.js";
import { listSubagents } from "../web/subagents.js";
export const Route = createFileRoute("/_authenticated/agents/$agentId/sub-agents/")({
  loader: ({ params }) => listSubagents({ data: { agentId: params.agentId } }), component: Page,
});
function Page() {
  const result = Route.useLoaderData(), { agentId } = Route.useParams();
  return result.ok ? <SubagentsPage agentId={agentId} definitions={result.data} /> : <p role="alert" className="text-red-300">{result.error}</p>;
}

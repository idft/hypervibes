import { createFileRoute } from "@tanstack/react-router";
import { SubagentsPage } from "../components/subagents-page.js";
import { subagentsQueryOptions } from "../web/subagent-queries.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/sub-agents/")({
  loader: async ({ params, context }) => { await context.queryClient.ensureQueryData(subagentsQueryOptions(params.agentId)); },
  component: Page,
});
function Page() {
  const { agentId } = Route.useParams();
  return <SubagentsPage key={agentId} agentId={agentId} />;
}

import { createFileRoute } from "@tanstack/react-router";
import { useSuspenseQuery } from "@tanstack/react-query";
import { SubagentRunPage } from "../components/subagent-run-page.js";
import { subagentRunQueryOptions } from "../web/subagent-queries.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/sub-agents/$subagentId_/runs/$runId")({
  loader: async ({ params, context }) => { await context.queryClient.fetchQuery(subagentRunQueryOptions(params.agentId, params.subagentId, params.runId)); },
  component: Page,
});
function Page() {
  const { agentId, subagentId, runId } = Route.useParams();
  const { data: initial } = useSuspenseQuery(subagentRunQueryOptions(agentId, subagentId, runId));
  return <SubagentRunPage key={`${agentId}:${runId}`} initial={initial} />;
}

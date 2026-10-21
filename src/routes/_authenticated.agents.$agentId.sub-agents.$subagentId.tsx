import { createFileRoute } from "@tanstack/react-router";
import { useSuspenseQuery } from "@tanstack/react-query";
import { SubagentDetailPage } from "../components/subagent-detail-page.js";
import { modelChoicesQueryOptions } from "../web/provider-queries.js";
import { subagentQueryOptions } from "../web/subagent-queries.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/sub-agents/$subagentId")({
  loader: async ({ params, context }) => {
    await Promise.all([context.queryClient.fetchQuery(subagentQueryOptions(params.agentId, params.subagentId)), context.queryClient.ensureQueryData(modelChoicesQueryOptions())]);
  },
  component: Page,
});
function Page() {
  const { agentId, subagentId } = Route.useParams();
  const { data: choices } = useSuspenseQuery(modelChoicesQueryOptions());
  return <SubagentDetailPage key={`${agentId}:${subagentId}`} agentId={agentId} subagentId={subagentId} choices={choices} />;
}

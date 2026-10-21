import { createFileRoute } from "@tanstack/react-router";
import { useSuspenseQuery } from "@tanstack/react-query";
import { AgentChartPage } from "../components/agent-chart-page.js";
import { agentChatQueryOptions } from "../web/agent-queries.js";
import { chartInstrumentsQueryOptions, chartLayoutsQueryOptions } from "../web/chart-queries.js";
import { modelChoicesQueryOptions } from "../web/provider-queries.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/chart")({
  loader: async ({ params, context }) => {
    await Promise.all([
      context.queryClient.ensureQueryData(chartInstrumentsQueryOptions(params.agentId)),
      context.queryClient.fetchQuery({ ...chartLayoutsQueryOptions(params.agentId), staleTime: 0 }),
      context.queryClient.fetchQuery(agentChatQueryOptions(params.agentId)),
      context.queryClient.ensureQueryData(modelChoicesQueryOptions()),
    ]);
  },
  component: ChartRoute,
});

function ChartRoute() {
  const { agentId } = Route.useParams();
  const { data: instruments, error } = useSuspenseQuery(chartInstrumentsQueryOptions(agentId));
  const { data: choices } = useSuspenseQuery(modelChoicesQueryOptions());
  const { data: chat } = useSuspenseQuery(agentChatQueryOptions(agentId));
  return <>{error && <p role="alert" className="text-sm text-amber-200">{error.message}</p>}<AgentChartPage key={agentId} id={agentId} instruments={instruments} initialChat={chat} choices={choices} /></>;
}

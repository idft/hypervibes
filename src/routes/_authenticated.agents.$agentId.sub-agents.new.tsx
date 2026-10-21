import { createFileRoute } from "@tanstack/react-router";
import { useSuspenseQuery } from "@tanstack/react-query";
import { NewSubagentPage } from "../components/new-subagent-page.js";
import { modelChoicesQueryOptions } from "../web/provider-queries.js";
import { subagentsQueryOptions } from "../web/subagent-queries.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/sub-agents/new")({
  loader: async ({ params, context }) => {
    await Promise.all([context.queryClient.ensureQueryData(subagentsQueryOptions(params.agentId)), context.queryClient.ensureQueryData(modelChoicesQueryOptions())]);
  },
  component: Page,
});
function Page() {
  const { agentId } = Route.useParams();
  const { data: definitions } = useSuspenseQuery(subagentsQueryOptions(agentId));
  const { data: choices } = useSuspenseQuery(modelChoicesQueryOptions());
  const main = definitions.find((entry) => entry.kind === "orchestrator");
  return main ? <NewSubagentPage key={agentId} agentId={agentId} model={main} choices={choices} /> : <p role="alert" className="text-red-300">Parent Agent model is unavailable. Open the Agent chat to initialize it, then retry.</p>;
}

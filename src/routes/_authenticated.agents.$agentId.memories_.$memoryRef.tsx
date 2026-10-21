import { createFileRoute } from "@tanstack/react-router";
import { useSuspenseQuery } from "@tanstack/react-query";
import { AgentMemoryDetailPage } from "../components/agent-memory-detail-page.js";
import { parseMemoryRef } from "../memory/memory-contracts.js";
import { memoryQueryOptions } from "../web/memory-queries.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/memories_/$memoryRef")({
  loader: async ({ params, context }) => {
    parseMemoryRef(params.memoryRef);
    await context.queryClient.ensureQueryData(memoryQueryOptions(params.agentId, params.memoryRef));
  },
  component: Page,
});
function Page() {
  const { agentId, memoryRef } = Route.useParams();
  const { data: memory, error } = useSuspenseQuery(memoryQueryOptions(agentId, memoryRef));
  return <>{error && <p role="alert" className="text-sm text-red-300">{error.message}</p>}<AgentMemoryDetailPage key={`${agentId}:${memory.ref}`} agentId={agentId} memory={memory} /></>;
}

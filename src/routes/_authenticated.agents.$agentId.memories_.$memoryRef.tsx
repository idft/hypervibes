import { createFileRoute, Link } from "@tanstack/react-router";
import { AgentMemoryDetailPage } from "../components/agent-memory-detail-page.js";
import { parseMemoryRef } from "../memory/memory-contracts.js";
import { getMemory } from "../web/memories.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/memories_/$memoryRef")({
  loader: ({ params }) => {
    parseMemoryRef(params.memoryRef);
    return getMemory({ data: { agentId: params.agentId, ref: params.memoryRef } });
  },
  component: Page,
});
function Page() {
  const { agentId } = Route.useParams();
  const result = Route.useLoaderData();
  return result.ok ? <AgentMemoryDetailPage key={`${agentId}:${result.data.ref}`} agentId={agentId} memory={result.data} /> : <section className="space-y-4 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6"><Link to="/agents/$agentId/memories" params={{ agentId }} search={{}} className="cursor-pointer text-sm text-zinc-400 hover:text-white">← Back to Memories</Link><p role="alert" className="text-sm text-red-300">{result.error}</p></section>;
}

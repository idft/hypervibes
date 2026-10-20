import { createFileRoute } from "@tanstack/react-router";
import { AgentMemoriesPage } from "../components/agent-memories-page.js";
import { parseMemoryRef } from "../memory/memory-contracts.js";
import { listSubagents } from "../web/subagents.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/memories")({
  validateSearch: (search: Record<string, unknown>): { memory?: string; scope?: string; version?: number } => {
    const result: { memory?: string; scope?: string; version?: number } = {};
    if (search.memory !== undefined) { if (typeof search.memory !== "string") throw new Error("Invalid memory reference"); parseMemoryRef(search.memory); result.memory = search.memory; }
    if (search.scope !== undefined) { if (typeof search.scope !== "string" || search.scope.length > 100) throw new Error("Invalid insight scope"); result.scope = search.scope; }
    if (search.version !== undefined) { const version = Number(search.version); if (!/^[1-9][0-9]*$/.test(String(search.version)) || !Number.isSafeInteger(version)) throw new Error("Invalid insight version"); result.version = version; }
    return result;
  },
  loader: ({ params }) => listSubagents({ data: { agentId: params.agentId } }),
  component: Page,
});
function Page() {
  const { agentId } = Route.useParams(), { memory, scope, version } = Route.useSearch(), workers = Route.useLoaderData();
  return workers.ok ? <AgentMemoriesPage key={`${agentId}:${memory ?? ""}:${scope === undefined ? "publications" : scope}:${version ?? ""}`} agentId={agentId} workers={workers.data} memory={memory} scopeId={scope} version={version} initialTab={memory ? "memories" : scope !== undefined || version !== undefined ? "insights" : "memories"} /> : <p role="alert" className="text-red-300">{workers.error}</p>;
}

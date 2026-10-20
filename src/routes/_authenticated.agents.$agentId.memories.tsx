import { createFileRoute } from "@tanstack/react-router";
import { AgentMemoriesPage } from "../components/agent-memories-page.js";
import { listSubagents } from "../web/subagents.js";
import { listMemories } from "../web/memories.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/memories")({
  validateSearch: (search: Record<string, unknown>): { scope?: string; version?: number } => {
    const result: { scope?: string; version?: number } = {};
    if (search.scope !== undefined) { if (typeof search.scope !== "string" || search.scope.length > 100) throw new Error("Invalid insight scope"); result.scope = search.scope; }
    if (search.version !== undefined) { const version = Number(search.version); if (!/^[1-9][0-9]*$/.test(String(search.version)) || !Number.isSafeInteger(version)) throw new Error("Invalid insight version"); result.version = version; }
    return result;
  },
  loaderDeps: ({ search }) => ({ insights: search.scope !== undefined || search.version !== undefined }),
  loader: async ({ params, deps }) => {
    const [workers, memories] = await Promise.all([
      listSubagents({ data: { agentId: params.agentId } }),
      deps.insights ? null : listMemories({ data: { agentId: params.agentId, filters: {} } }),
    ]);
    return { workers, memories };
  },
  pendingComponent: () => <section role="status" className="w-full space-y-4 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6"><p className="text-sm text-zinc-400">Loading memories…</p><div aria-hidden="true" className="h-48 animate-pulse rounded-xl bg-zinc-900" /></section>,
  component: Page,
});
function Page() {
  const { agentId } = Route.useParams(), { scope, version } = Route.useSearch(), { workers, memories } = Route.useLoaderData();
  if (!workers.ok) return <p role="alert" className="text-red-300">{workers.error}</p>;
  if (memories && !memories.ok) return <p role="alert" className="text-red-300">{memories.error}</p>;
  return <AgentMemoriesPage key={`${agentId}:${scope === undefined ? "publications" : scope}:${version ?? ""}`} agentId={agentId} workers={workers.data} initialMemories={memories?.ok ? memories.data : undefined} scopeId={scope} version={version} initialTab={scope !== undefined || version !== undefined ? "insights" : "memories"} />;
}

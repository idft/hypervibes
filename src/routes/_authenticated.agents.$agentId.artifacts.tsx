import { createFileRoute } from "@tanstack/react-router";
import { AgentArtifactsPage } from "../components/agent-artifacts-page.js";
import { listArtifacts, readArtifact } from "../web/artifacts.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/artifacts")({
  validateSearch: (search: Record<string, unknown>) => ({ artifact: typeof search.artifact === "string" && /^[a-f0-9-]{36}$/.test(search.artifact) ? search.artifact : undefined }),
  loaderDeps: ({ search }) => ({ artifact: search.artifact }),
  loader: async ({ params, deps }) => {
    const page = await listArtifacts({ data: { agentId: params.agentId, query: "" } });
    if (!page.ok) throw new Error(page.error);
    const preview = deps.artifact ? await readArtifact({ data: { agentId: params.agentId, id: deps.artifact } }) : null;
    return { page: page.data, preview: preview?.ok ? preview.data : null, previewError: preview && !preview.ok ? preview.error : undefined };
  },
  component: function Artifacts() {
    const { agentId } = Route.useParams(), { page, preview, previewError } = Route.useLoaderData();
    return <AgentArtifactsPage key={`${agentId}:${preview?.id ?? ""}`} agentId={agentId} initial={page} initialArtifact={preview} initialError={previewError} />;
  },
});

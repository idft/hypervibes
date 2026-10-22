import { createFileRoute } from "@tanstack/react-router";
import { IndicatorDetailPage } from "../components/indicator-detail-page.js";
import { indicatorQueryOptions } from "../web/indicator-queries.js";
import { parseIndicatorId } from "../indicators/validation.js";
export const Route = createFileRoute("/_authenticated/agents/$agentId/indicators_/$indicatorId")({
  validateSearch: (value: Record<string, unknown>): { copy?: string } => { if (typeof value.copy === "string") { parseIndicatorId(value.copy); return { copy: value.copy }; } return {}; },
  loaderDeps: ({ search }) => search,
  loader: async ({ context, params, deps }) => { const id = params.indicatorId === "new" ? deps.copy : params.indicatorId; if (id) { parseIndicatorId(id); await context.queryClient.ensureQueryData(indicatorQueryOptions(params.agentId, id)); } },
  component: Page,
});
function Page() { const { agentId, indicatorId } = Route.useParams(); const { copy } = Route.useSearch(); return <IndicatorDetailPage key={`${agentId}:${indicatorId}:${copy ?? ""}`} agentId={agentId} indicatorId={indicatorId} copy={copy} />; }

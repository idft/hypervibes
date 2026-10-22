import { createFileRoute } from "@tanstack/react-router";
import { AgentIndicatorsPage, type IndicatorSearch } from "../components/agent-indicators-page.js";
import { indicatorListQueryOptions } from "../web/indicator-queries.js";
export const Route = createFileRoute("/_authenticated/agents/$agentId/indicators")({
  validateSearch: (value: Record<string, unknown>): IndicatorSearch => ({ query: typeof value.query === "string" ? value.query.slice(0, 160) : "", kind: value.kind === "builtin" || value.kind === "custom" ? value.kind : "all", offset: typeof value.offset === "number" && Number.isSafeInteger(value.offset) && value.offset >= 0 ? value.offset : 0 }),
  loaderDeps: ({ search }) => search,
  loader: ({ context, params, deps }) => {
    // Warm the cache without blocking navigation; the page renders loading/errors.
    void context.queryClient.prefetchQuery(indicatorListQueryOptions(params.agentId, deps));
  },
  component: Page,
});
function Page() { const { agentId } = Route.useParams(); const search = Route.useSearch(), navigate = Route.useNavigate(); return <AgentIndicatorsPage agentId={agentId} search={search} onSearch={(search) => { void navigate({ search, replace: true }); }} />; }

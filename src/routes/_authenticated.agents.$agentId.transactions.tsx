import { createFileRoute } from "@tanstack/react-router";
import { AgentJournalPage } from "../components/agent-journal-page.js";
import { journalQueryOptions } from "../web/journal-queries.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/transactions")({
  validateSearch: (search: Record<string, unknown>): { view: "trades" | "activity"; page: number } => ({
    view: search.view === "activity" ? "activity" : "trades",
    page: /^\d{1,8}$/.test(String(search.page)) && Number(search.page) > 0 ? Number(search.page) : 1,
  }),
  loaderDeps: ({ search }) => search,
  loader: ({ context: { queryClient }, params, deps }) => queryClient.ensureQueryData(journalQueryOptions(params.agentId, deps.view, deps.page)),
  component: JournalRoute,
});
function JournalRoute() {
  const { agentId } = Route.useParams();
  const { view, page } = Route.useSearch();
  const navigate = Route.useNavigate();
  return <AgentJournalPage key={agentId} id={agentId} view={view} page={page} onNavigate={(view, page) => { void navigate({ search: { view, page } }); }} />;
}
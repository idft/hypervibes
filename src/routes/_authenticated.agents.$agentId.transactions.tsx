import { createFileRoute } from "@tanstack/react-router";
import { AgentJournalPage } from "../components/agent-journal-page.js";
import { getAgentJournal } from "../web/journal.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/transactions")({
  validateSearch: (search: Record<string, unknown>): { view: "trades" | "activity"; page: number } => ({
    view: search.view === "activity" ? "activity" : "trades",
    page: /^\d{1,8}$/.test(String(search.page)) && Number(search.page) > 0 ? Number(search.page) : 1,
  }),
  loaderDeps: ({ search }) => search,
  loader: ({ params, deps }) => getAgentJournal({ data: { id: params.agentId, view: deps.view, page: String(deps.page) } }),
  component: JournalRoute,
});
function JournalRoute() {
  const result = Route.useLoaderData();
  const { agentId } = Route.useParams();
  const navigate = Route.useNavigate();
  return result.ok ? <AgentJournalPage key={agentId} id={agentId} initial={result.data} onNavigate={(view, page) => { void navigate({ search: { view, page } }); }} /> : <p role="alert" className="text-red-300">{result.error}</p>;
}

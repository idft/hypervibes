import { createFileRoute } from "@tanstack/react-router";
import { AgentNotificationsPage } from "../components/agent-notifications-page.js";
import { notificationsQueryOptions } from "../web/notification-queries.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/notifications")({
  validateSearch: (value: Record<string, unknown>) => ({ page: typeof value.page === "number" && Number.isSafeInteger(value.page) && value.page >= 1 && value.page <= 100000 ? value.page : 1 }),
  loaderDeps: ({ search }) => search,
  loader: ({ context, params, deps }) => { void context.queryClient.query(notificationsQueryOptions(params.agentId, deps.page)).catch(() => undefined); },
  component: Page,
});
function Page() {
  const { agentId } = Route.useParams(), { page } = Route.useSearch(), navigate = Route.useNavigate();
  return <AgentNotificationsPage agentId={agentId} page={page} onPage={(page) => { void navigate({ search: { page } }); }} />;
}

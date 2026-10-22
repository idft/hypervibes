import { createFileRoute } from "@tanstack/react-router";
import { AgentNotificationDetailPage } from "../components/agent-notifications-page.js";
import { notificationQueryOptions } from "../web/notification-queries.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/notifications_/$notificationId")({
  loader: async ({ context, params }) => { await context.queryClient.query({ ...notificationQueryOptions(params.agentId, params.notificationId), staleTime: "static" }); },
  component: Page,
});
function Page() { const { agentId, notificationId } = Route.useParams(); return <AgentNotificationDetailPage agentId={agentId} notificationId={notificationId} />; }

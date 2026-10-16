import { createFileRoute } from "@tanstack/react-router";
import { AgentSectionPage } from "../components/agent-section-page.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/chart")({
  component: () => <AgentSectionPage title="Chart" description="Market charts and indicators will be available here." />,
});

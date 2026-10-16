import { createFileRoute } from "@tanstack/react-router";
import { AgentSectionPage } from "../components/agent-section-page.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/memories")({
  component: () => <AgentSectionPage title="Memories" description="Agent memories will be available here." />,
});

import { createFileRoute } from "@tanstack/react-router";
import { AgentSectionPage } from "../components/agent-section-page.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/sub-agents")({
  component: () => <AgentSectionPage title="Sub-agents" description="Delegated Analysis, Trading, and Review work will be available here." />,
});

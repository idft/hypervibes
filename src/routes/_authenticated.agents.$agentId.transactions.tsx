import { createFileRoute } from "@tanstack/react-router";
import { AgentSectionPage } from "../components/agent-section-page.js";

export const Route = createFileRoute("/_authenticated/agents/$agentId/transactions")({
  component: () => <AgentSectionPage title="Transactions" description="Account transactions and trade history will be available here." />,
});

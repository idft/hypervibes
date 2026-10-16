import { createFileRoute } from "@tanstack/react-router";
import { CreateAgentPage } from "../components/create-agent-page.js";
import { createAgent, createAgentSubaccount, getTradingAccountChoices } from "../web/agents.js";

export const Route = createFileRoute("/_authenticated/agents/new")({
  loader: () => getTradingAccountChoices(),
  component: CreateAgentRoute,
});

function CreateAgentRoute() {
  return (
    <CreateAgentPage
      choices={Route.useLoaderData()}
      create={(name, requestId, accountSelection) => createAgent({ data: { name, requestId, accountSelection } })}
      createSubaccount={(name) => createAgentSubaccount({ data: { name } })}
    />
  );
}
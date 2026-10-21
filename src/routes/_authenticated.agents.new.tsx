import { createFileRoute, redirect } from "@tanstack/react-router";
import { CreateAgentPage } from "../components/create-agent-page.js";
import { createAgent, createAgentSubaccount, getTradingAccountChoices } from "../web/agents.js";

import { getModelChoices } from "../web/providers.js";

export const Route = createFileRoute("/_authenticated/agents/new")({
  loader: async () => {
    const models = await getModelChoices();
    if (!models.length) throw redirect({ to: "/providers" });
    return { choices: await getTradingAccountChoices(), models };
  },
  component: CreateAgentRoute,
});

function CreateAgentRoute() {
  return (
    <CreateAgentPage
      choices={Route.useLoaderData().choices}
      models={Route.useLoaderData().models}
      create={(name, requestId, accountSelection, model, includeDefaultSubagents, thinkingLevel) => createAgent({ data: { name, requestId, accountSelection, provider: model.provider, modelId: model.modelId, includeDefaultSubagents, thinkingLevel } })}
      createSubaccount={(name) => createAgentSubaccount({ data: { name } })}
    />
  );
}
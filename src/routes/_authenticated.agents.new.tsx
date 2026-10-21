import { createFileRoute, redirect } from "@tanstack/react-router";
import { CreateAgentPage } from "../components/create-agent-page.js";
import { createAgent, createAgentSubaccount } from "../web/agents.js";

import { useSuspenseQuery } from "@tanstack/react-query";
import { modelChoicesQueryOptions } from "../web/provider-queries.js";
import { tradingAccountsQueryOptions } from "../web/agent-queries.js";

export const Route = createFileRoute("/_authenticated/agents/new")({
  loader: async ({ context }) => {
    // This guard needs the refreshed catalog, not an invalidated cached empty list.
    const models = await context.queryClient.fetchQuery(modelChoicesQueryOptions());
    if (!models.length) throw redirect({ to: "/providers" });
    await context.queryClient.ensureQueryData(tradingAccountsQueryOptions());
  },
  component: CreateAgentRoute,
});

function CreateAgentRoute() {
  const { data: models } = useSuspenseQuery(modelChoicesQueryOptions());
  const { data: choices } = useSuspenseQuery(tradingAccountsQueryOptions());
  return (
    <CreateAgentPage
      choices={choices}
      models={models}
      create={(name, requestId, accountSelection, model, includeDefaultSubagents, thinkingLevel) => createAgent({ data: { name, requestId, accountSelection, provider: model.provider, modelId: model.modelId, includeDefaultSubagents, thinkingLevel } })}
      createSubaccount={(name) => createAgentSubaccount({ data: { name } })}
    />
  );
}
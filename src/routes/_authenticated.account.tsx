import { createFileRoute } from "@tanstack/react-router";
import { AccountPage } from "../components/account-page.js";
import { listAgents } from "../web/agents.js";
import { getProviderSettings } from "../web/providers.js";
import { approveApiWallet, approveBuilderFee, cancelBuilderFee, getAccount, setupApiWallet } from "../web/account.js";

export const Route = createFileRoute("/_authenticated/account")({
  loader: async () => {
    const [view, agents, providers] = await Promise.all([getAccount(), listAgents(), getProviderSettings()]);
    return { view, setup: { agentCount: agents.length, providerConnected: providers.providers.some((provider) => provider.connected) } };
  },
  component: AccountRoute,
});

function AccountRoute() {
  const { view, setup } = Route.useLoaderData();
  return (
    <AccountPage
      view={view}
      setup={setup}
      actions={{
        setupApiWallet: (source, privateKey) => setupApiWallet({ data: { source, privateKey } }),
        approveApiWallet: (request) => approveApiWallet({ data: request }),
        approveBuilderFee: (request, feeBps) => approveBuilderFee({ data: { ...request as object, feeBps } }),
        cancelBuilderFee: (request) => cancelBuilderFee({ data: request }),
      }}
    />
  );
}
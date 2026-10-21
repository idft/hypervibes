import { createFileRoute } from "@tanstack/react-router";
import { useSuspenseQuery } from "@tanstack/react-query";
import { AccountPage } from "../components/account-page.js";
import { agentsQueryOptions } from "../web/agent-queries.js";
import { providerSettingsQueryOptions } from "../web/provider-queries.js";
import { approveApiWallet, approveBuilderFee, cancelBuilderFee, setupApiWallet } from "../web/account.js";
import { accountQueryOptions } from "../web/account-queries.js";

export const Route = createFileRoute("/_authenticated/account")({
  loader: async ({ context }) => {
    await Promise.all([context.queryClient.ensureQueryData(accountQueryOptions()), context.queryClient.ensureQueryData(agentsQueryOptions()), context.queryClient.ensureQueryData(providerSettingsQueryOptions())]);
  },
  component: AccountRoute,
});

function AccountRoute() {
  const { data: view, error } = useSuspenseQuery(accountQueryOptions());
  const { data: agents } = useSuspenseQuery(agentsQueryOptions());
  const { data: providers } = useSuspenseQuery(providerSettingsQueryOptions());
  const setup = { agentCount: agents.length, providerConnected: providers.providers.some((provider) => provider.connected) };
  return (
    <>
      {error && <p role="alert" className="mb-4 text-sm text-red-300">{error.message}</p>}
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
    </>
  );
}
import { createFileRoute } from "@tanstack/react-router";
import { AccountPage } from "../components/account-page.js";
import { approveApiWallet, approveBuilderFee, cancelBuilderFee, getAccount, setupApiWallet } from "../web/account.js";

export const Route = createFileRoute("/_authenticated/account")({
  loader: () => getAccount(),
  component: AccountRoute,
});

function AccountRoute() {
  return (
    <AccountPage
      view={Route.useLoaderData()}
      actions={{
        setupApiWallet: (source, privateKey) => setupApiWallet({ data: { source, privateKey } }),
        approveApiWallet: (request) => approveApiWallet({ data: request }),
        approveBuilderFee: (request, feeBps) => approveBuilderFee({ data: { ...request as object, feeBps } }),
        cancelBuilderFee: (request) => cancelBuilderFee({ data: request }),
      }}
    />
  );
}
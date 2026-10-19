import { createFileRoute } from "@tanstack/react-router";
import { ProvidersPage } from "../components/providers-page.js";
import { getProviderSettings } from "../web/providers.js";
import { getSetupState } from "../web/setup.js";

export const Route = createFileRoute("/_authenticated/providers")({
  loader: async () => {
    const [settings, setup] = await Promise.all([getProviderSettings(), getSetupState()]);
    return { settings, setup };
  },
  component: ProvidersRoute,
});

function ProvidersRoute() {
  const { settings, setup } = Route.useLoaderData();
  return <ProvidersPage initial={settings} agentCount={setup.agentCount} setup={setup} />;
}

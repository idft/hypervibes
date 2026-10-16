import { createFileRoute } from "@tanstack/react-router";
import { ProvidersPage } from "../components/providers-page.js";
import { getProviderSettings } from "../web/providers.js";

export const Route = createFileRoute("/_authenticated/providers")({
  loader: () => getProviderSettings(),
  component: () => <ProvidersPage initial={Route.useLoaderData()} />,
});

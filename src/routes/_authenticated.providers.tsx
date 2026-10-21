import { createFileRoute } from "@tanstack/react-router";
import { ProvidersPage } from "../components/providers-page.js";
import { providerSettingsQueryOptions, setupQueryOptions } from "../web/provider-queries.js";

export const Route = createFileRoute("/_authenticated/providers")({
  loader: async ({ context }) => {
    await Promise.all([
      context.queryClient.ensureQueryData(providerSettingsQueryOptions()),
      context.queryClient.ensureQueryData(setupQueryOptions()),
    ]);
  },
  component: ProvidersPage,
});

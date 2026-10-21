import { mutationOptions, queryOptions, type QueryClient } from "@tanstack/react-query";
import type { AuthType } from "@earendil-works/pi-ai";
import type { LoginView } from "../providers/provider-service.js";
import { answerProviderLogin, beginProviderLogin, cancelProviderLogin, disconnectProvider, getLoginDestination, getModelChoices, getProviderSettings, refreshProviderModels } from "./providers.js";
import { getSetupState } from "./setup.js";

export const loginDestinationQueryOptions = () => queryOptions({
  queryKey: ["login-destination"],
  queryFn: () => getLoginDestination(),
  // This is a post-auth decision, never reuse another login's readiness.
  staleTime: 0,
  gcTime: 0,
});

export const providerSettingsQueryOptions = () => queryOptions({
  queryKey: ["providers", "settings"],
  queryFn: () => getProviderSettings(),
});
export const modelChoicesQueryOptions = () => queryOptions({
  queryKey: ["providers", "models"],
  queryFn: () => getModelChoices(),
});
export const setupQueryOptions = () => queryOptions({
  queryKey: ["setup"],
  queryFn: () => getSetupState(),
});

export async function invalidateProviderQueries(client: QueryClient) {
  await Promise.all([
    client.invalidateQueries({ queryKey: ["providers"] }),
    client.invalidateQueries({ queryKey: setupQueryOptions().queryKey }),
  ]);
}

export type ProviderOperation =
  | { type: "begin"; providerId: string; authType: AuthType }
  | { type: "answer"; id: string; promptId: string; value: string }
  | { type: "cancel"; id: string }
  | { type: "disconnect"; providerId: string }
  | { type: "refresh" };

/** One mutation owns this page's pending/error state; login progress still uses SSE. */
export const providerMutationOptions = (client: QueryClient) => mutationOptions<LoginView | undefined, Error, ProviderOperation>({
  mutationFn: async (operation: ProviderOperation) => {
    switch (operation.type) {
      case "begin": return beginProviderLogin({ data: { providerId: operation.providerId, type: operation.authType } });
      case "answer": await answerProviderLogin({ data: { id: operation.id, promptId: operation.promptId, value: operation.value } }); return;
      case "cancel": await cancelProviderLogin({ data: { id: operation.id } }); return;
      case "disconnect": await disconnectProvider({ data: { providerId: operation.providerId } }); return;
      case "refresh": await refreshProviderModels(); return;
    }
  },
  onSettled: async (login, _error, operation) => {
    // A partially successful refresh can change the catalog even when it throws.
    // Pending login/answer snapshots are reconciled by the stream, not refetches.
    if (operation.type === "answer" || (operation.type === "begin" && login?.state === "pending")) return;
    await invalidateProviderQueries(client);
  },
});

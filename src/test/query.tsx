import { QueryClient, QueryClientProvider, type DefaultOptions } from "@tanstack/react-query";
import type { ComponentType } from "react";
import { onTestFinished } from "vitest";

/** A fresh cache per test/SSR render, cleared after the test's UI teardown. */
export function createTestQueryClient(queries: DefaultOptions["queries"] = {}) {
  const client = new QueryClient({ defaultOptions: {
    queries: { retry: false, gcTime: Infinity, ...queries },
    mutations: { retry: false },
  } });
  onTestFinished(() => client.clear());
  return client;
}

/** Resolve the client at render time, after the test's beforeEach has run. */
export function withTestQueryClient<Props extends object>(Component: ComponentType<Props>, getClient: () => QueryClient) {
  return function QueryTestComponent(props: Props) {
    return <QueryClientProvider client={getClient()}><Component {...props} /></QueryClientProvider>;
  };
}

import { QueryClient } from "@tanstack/react-query";
import { createRouter } from "@tanstack/react-router";
import { setupRouterSsrQueryIntegration } from "@tanstack/react-router-ssr-query";
import type { AppRuntime } from "./runtime.js";
import { RouteErrorPage } from "./components/route-status-page.js";
import { routeTree } from "./routeTree.gen.js";

export function getRouter() {
  // One client per SSR request on the server (the SSR integration clears the
  // cache after each response) and one per page load in the browser.
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        refetchOnWindowFocus: false,
        retry: false,
        staleTime: 30_000,
      },
    },
  });
  const router = createRouter({
    routeTree,
    scrollRestoration: true,
    defaultErrorComponent: RouteErrorPage,
    context: { queryClient },
  });
  // Dehydrates the query cache with the server-rendered page and wraps the
  // router output in a QueryClientProvider on both server and client.
  setupRouterSsrQueryIntegration({ router, queryClient });
  return router;
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof getRouter>;
    server: { requestContext: { runtime: AppRuntime } };
  }
}

// Start's Vite-generated registration must retain the host-injected context too.
declare module "@tanstack/react-start" {
  interface Register {
    server: { requestContext: { runtime: AppRuntime } };
  }
}
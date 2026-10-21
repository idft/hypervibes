import { createRouter } from "@tanstack/react-router";
import type { AppRuntime } from "./runtime.js";
import { RouteErrorPage } from "./components/route-status-page.js";
import { routeTree } from "./routeTree.gen.js";

export function getRouter() {
  return createRouter({ routeTree, scrollRestoration: true, defaultErrorComponent: RouteErrorPage });
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

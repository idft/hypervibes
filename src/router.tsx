import { createRouter } from "@tanstack/react-router";
import type { AppRuntime } from "./runtime.js";
import { routeTree } from "./routeTree.gen.js";

export function getRouter() {
  return createRouter({ routeTree, scrollRestoration: true });
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof getRouter>;
    server: { requestContext: { runtime: AppRuntime } };
  }
}

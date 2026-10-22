// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ beforeLoad: vi.fn() }));
vi.mock("./routeTree.gen.js", async () => {
  const { createRootRouteWithContext, createRoute } = await import("@tanstack/react-router");
  const root = createRootRouteWithContext()();
  return { routeTree: root.addChildren([
    createRoute({ getParentRoute: () => root, path: "/", component: () => null }),
    createRoute({ getParentRoute: () => root, path: "/indicators", beforeLoad: () => mocks.beforeLoad(), component: () => null }),
  ]) };
});
vi.mock("@tanstack/react-router-ssr-query", () => ({ setupRouterSsrQueryIntegration: vi.fn() }));
import { getRouter } from "./router.js";

beforeEach(() => { mocks.beforeLoad.mockReset(); window.history.replaceState(null, "", "/"); });

it("aborts chart indicator requests before destination route work begins", async () => {
  const router = getRouter();
  await router.load();
  const client = router.options.context!.queryClient;
  let requestSignal: AbortSignal | undefined;
  const query = client.fetchQuery({
    queryKey: ["indicators", "agent", "render", "layout"],
    queryFn: ({ signal }) => {
      requestSignal = signal;
      return new Promise(() => {});
    },
  }).catch(() => {});
  expect(requestSignal?.aborted).toBe(false);
  mocks.beforeLoad.mockImplementation(() => { expect(requestSignal?.aborted).toBe(true); });
  // The test route tree is intentionally smaller than the application's registered tree.
  await router.navigate({ href: "/indicators" });
  await query;
  expect(mocks.beforeLoad).toHaveBeenCalledOnce();
  expect(client.isFetching()).toBe(0);
  client.clear();
});

it("does not cancel indicator calculations for same-page search changes", async () => {
  const router = getRouter();
  await router.load();
  const client = router.options.context!.queryClient;
  let requestSignal: AbortSignal | undefined;
  const query = client.fetchQuery({ queryKey: ["indicators", "agent", "render", "layout"], queryFn: ({ signal }) => { requestSignal = signal; return new Promise(() => {}); } }).catch(() => {});
  await router.navigate({ href: "/?view=other" });
  expect(requestSignal?.aborted).toBe(false);
  await client.cancelQueries(); await query; client.clear();
});

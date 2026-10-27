import { useEffect } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryHistory, createRootRoute, createRoute, createRouter, Outlet } from "@tanstack/react-router";
import { Navbar } from "../components/navbar.js";
import { AgentLayout } from "../components/agent-layout.js";
import { AgentDashboardPage } from "../components/agent-dashboard-page.js";
import { dashboardFixture, screenshotAddress } from "./dashboard-fixture.js";

/** Separate router: never added to the application's generated route tree. */
export function createDashboardScreenshotRouter(btcPrice: string) {
  const fixture = dashboardFixture(btcPrice);
  const client = new QueryClient({ defaultOptions: { queries: { enabled: false, retry: false, refetchOnWindowFocus: false } } });
  function Shell() {
    return <QueryClientProvider client={client}>
      <div inert>
        <header className="fixed inset-x-0 top-0 z-40 h-14 border-b border-zinc-800 bg-[#17191c]">
          <Navbar walletAddress={screenshotAddress} agents={[fixture.agent]} selectedAgentId={fixture.agent.id} />
        </header>
        <div className="app-shell fixed inset-x-0 bottom-0 top-14 overflow-y-auto">
          <main className="app-content min-w-0 p-2 sm:p-3"><Outlet /></main>
        </div>
      </div>
    </QueryClientProvider>;
  }
  function Dashboard() {
    useEffect(() => {
      document.documentElement.dataset.screenshotReady = "true";
      return () => { delete document.documentElement.dataset.screenshotReady; };
    }, []);
    return <AgentDashboardPage id={fixture.agent.id} initial={fixture.dashboard} subagents={fixture.subagents} />;
  }
  const root = createRootRoute({ component: Shell });
  const screenshot = createRoute({ getParentRoute: () => root, path: "/agents/$agentId",
    component: () => <AgentLayout agent={fixture.agent} initialExpanded /> });
  const dashboard = createRoute({ getParentRoute: () => screenshot, path: "/", component: Dashboard });
  // Keep native active navigation without changing the temporary server's /screenshot URL.
  return createRouter({ routeTree: root.addChildren([screenshot.addChildren([dashboard])]),
    history: createMemoryHistory({ initialEntries: [`/agents/${fixture.agent.id}`] }), defaultPreload: false });
}

import { createFileRoute, Outlet, redirect, useParams } from "@tanstack/react-router";
import { Navbar } from "../components/navbar.js";
import { getCurrentUser } from "../web/auth.js";
import { listAgents } from "../web/agents.js";

export const Route = createFileRoute("/_authenticated")({
  beforeLoad: async () => {
    const user = await getCurrentUser();
    if (!user) throw redirect({ to: "/login" });
    return { user };
  },
  loader: () => listAgents(),
  component: AuthenticatedLayout,
});

function AuthenticatedLayout() {
  const { user } = Route.useRouteContext();
  const agents = Route.useLoaderData();
  const { agentId } = useParams({ strict: false });
  return (
    <>
      <header className="fixed inset-x-0 top-0 z-40 h-14 border-b border-zinc-800 bg-[#17191c]">
        <Navbar walletAddress={user.walletAddress} agents={agents} selectedAgentId={agentId} />
      </header>
      <div className="app-shell fixed inset-x-0 bottom-0 top-14 overflow-y-auto">
        <main className={`app-content min-w-0 ${agentId ? "p-2 sm:p-3" : "px-4 py-6 sm:px-6 lg:px-8"}`}><Outlet /></main>
      </div>
    </>
  );
}

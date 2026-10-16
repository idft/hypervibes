import { createFileRoute, Outlet } from "@tanstack/react-router";
export const Route = createFileRoute("/_authenticated/agents/$agentId/sub-agents")({ component: Outlet });

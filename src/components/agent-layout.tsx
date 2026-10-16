import { Link, Outlet } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import type { AgentRow } from "../db/agent-schema.js";

const sections = [
  { label: "Dashboard", to: "/agents/$agentId", icon: "M3 3h6v6H3zM13 3h6v6h-6zM3 13h6v6H3zM13 13h6v6h-6z" },
  { label: "Chat", to: "/agents/$agentId/chat", icon: "M4 4h16v12H9l-5 4z" },
  { label: "Transactions", to: "/agents/$agentId/transactions", icon: "M3 7h17m-4-4 4 4-4 4M21 17H4m4-4-4 4 4 4" },
  { label: "Chart", to: "/agents/$agentId/chart", icon: "M3 3v18h18M6 15l5-6 4 3 5-7" },
  { label: "Memories", to: "/agents/$agentId/memories", icon: "M4 4h16v16H4zM8 8h8M8 12h8M8 16h5" },
  { label: "Sub-agents", to: "/agents/$agentId/sub-agents", icon: "M9 3h6v6H9zM3 16h6v6H3zM15 16h6v6h-6zM12 9v4M6 16v-3h12v3" },
  { label: "Settings", to: "/agents/$agentId/settings", icon: "M4 6h16M4 12h16M4 18h16M8 3v6M16 9v6M10 15v6" },
] as const;

export function AgentLayout({ agent }: { agent: AgentRow }) {
  const [expanded, setExpanded] = useState(false);
  useEffect(() => {
    try { setExpanded(localStorage.getItem("agent-navigation-expanded") === "true"); } catch { /* Storage is optional. */ }
  }, []);
  function toggle() {
    setExpanded((value) => {
      try { localStorage.setItem("agent-navigation-expanded", String(!value)); } catch { /* Storage is optional. */ }
      return !value;
    });
  }
  const link = (section: typeof sections[number]) => <Link key={section.label} to={section.to} params={{ agentId: agent.id }} activeOptions={{ exact: true }} title={section.label} aria-label={section.label}
    className="flex h-10 cursor-pointer items-center gap-3 rounded-lg px-3 text-zinc-400 hover:bg-zinc-800 hover:text-white"
    activeProps={{ className: "bg-zinc-800 text-white", "aria-current": "page" }}>
    <svg className="h-5 w-5 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={section.icon} /></svg>
    {expanded && <span className="text-sm">{section.label}</span>}
  </Link>;
  return <div className={expanded ? "pl-48" : "pl-14 sm:pl-16"}>
    <nav aria-label="Agent sections" className={`fixed bottom-0 left-0 top-14 z-30 flex flex-col border-r border-zinc-800 bg-[#111214] p-1.5 ${expanded ? "w-48" : "w-14 sm:w-16"}`}>
      <div className="flex flex-col gap-1">{sections.slice(0, -1).map(link)}</div>
      <div className="mt-auto space-y-2">
        <button type="button" onClick={toggle} aria-expanded={expanded} aria-label={expanded ? "Collapse agent navigation" : "Expand agent navigation"} title={expanded ? "Collapse agent navigation" : "Expand agent navigation"} className="flex h-10 w-full cursor-pointer items-center gap-3 rounded-lg px-3 text-zinc-400 hover:bg-zinc-800 hover:text-white"><span aria-hidden="true">{expanded ? "«" : "»"}</span>{expanded && <span className="text-sm">Collapse</span>}</button>
        <div className="border-t border-zinc-800 pt-2">{link(sections[6])}</div>
      </div>
    </nav>
    <div className="flex min-h-[calc(100dvh-5rem)] w-full min-w-0 flex-col">
      <Outlet />
    </div>
  </div>;
}

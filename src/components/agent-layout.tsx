import { Link, Outlet } from "@tanstack/react-router";
import { useState } from "react";
import type { AgentRow } from "../db/agent-schema.js";

const sections = [
  { label: "Dashboard", to: "/agents/$agentId", icon: "M3 3h6v6H3zM13 3h6v6h-6zM3 13h6v6H3zM13 13h6v6h-6z" },
  { label: "Charts", to: "/agents/$agentId/chart", icon: "M3 3v18h18M6 15l5-6 4 3 5-7" },
  { label: "Transactions", to: "/agents/$agentId/transactions", icon: "M3 7h17m-4-4 4 4-4 4M21 17H4m4-4-4 4 4 4" },
  { label: "Chat", to: "/agents/$agentId/chat", icon: "M4 4h16v12H9l-5 4z" },
  { label: "Sub-Agents", to: "/agents/$agentId/sub-agents", icon: "M9 3h6v6H9zM3 16h6v6H3zM15 16h6v6h-6zM12 9v4M6 16v-3h12v3" },
  { label: "Settings", to: "/agents/$agentId/settings", icon: "M4 6h16M4 12h16M4 18h16M8 3v6M16 9v6M10 15v6" },
] as const;

const memorySections = [
  { label: "Insights", to: "/agents/$agentId/memories", search: { scope: "" }, icon: "M4 4h16v16H4zM8 8h8M8 12h8M8 16h5" },
  { label: "Artifacts", to: "/agents/$agentId/artifacts", icon: "M4 3h10l6 6v12H4zM14 3v6h6M8 13h8M8 17h5" },
] as const;

export function AgentLayout({ agent, initialExpanded }: { agent: AgentRow; initialExpanded: boolean }) {
  const [expanded, setExpanded] = useState(initialExpanded);
  const [memoriesExpanded, setMemoriesExpanded] = useState(false);
  function toggle() {
    setExpanded((value) => {
      document.cookie = `hv-agent-nav-expanded=${!value}; Max-Age=31536000; Path=/; SameSite=Strict`;
      return !value;
    });
  }
  const link = (section: typeof sections[number] | typeof memorySections[number]) => <Link key={section.label} to={section.to} params={{ agentId: agent.id }} search={"search" in section ? section.search : undefined} activeOptions={{ exact: true }} title={section.label} aria-label={section.label}
    className="flex h-10 cursor-pointer items-center gap-3 rounded-lg px-3 text-zinc-400 hover:bg-zinc-800 hover:text-white"
    activeProps={{ className: "bg-zinc-800 text-white", "aria-current": "page" }}>
    <svg className="h-5 w-5 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={section.icon} /></svg>
    {expanded && <span className="text-sm">{section.label}</span>}
  </Link>;
  return <div className={expanded ? "pl-48" : "pl-14 sm:pl-16"}>
    <nav aria-label="Agent sections" className={`fixed bottom-0 left-0 top-14 z-30 flex flex-col border-r border-zinc-800 bg-[#111214] p-1.5 ${expanded ? "w-48" : "w-14 sm:w-16"}`}>
      <div className="flex min-h-0 flex-col gap-1 overflow-y-auto">
        {sections.slice(0, 3).map(link)}
        <hr className="my-2 border-zinc-800" />
        {sections.slice(3, -1).map(link)}
        <Link to="/agents/$agentId/memories" params={{ agentId: agent.id }} search={{}} onClick={() => setMemoriesExpanded(true)} activeOptions={{ exact: true, includeSearch: true }} activeProps={{ className: "bg-zinc-800 text-white", "aria-current": "page" }} aria-expanded={memoriesExpanded} aria-controls="agent-memory-sections" aria-label="Memories" title="Memories" className="flex h-10 shrink-0 cursor-pointer items-center gap-3 rounded-lg px-3 text-zinc-400 hover:bg-zinc-800 hover:text-white">
          <svg className="h-5 w-5 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M4 4h16v16H4zM8 8h8M8 12h8M8 16h5" /></svg>
          {expanded && <><span className="text-sm">Memories</span><span className="ml-auto text-xs" aria-hidden="true">{memoriesExpanded ? "▾" : "▸"}</span></>}
        </Link>
        <div id="agent-memory-sections" hidden={!memoriesExpanded} className={expanded ? "ml-4 space-y-1 border-l border-zinc-800 pl-1" : "space-y-1"}>{memoriesExpanded && memorySections.map(link)}</div>
      </div>
      <div className="mt-auto space-y-2">
        <button type="button" onClick={toggle} aria-expanded={expanded} aria-label={expanded ? "Collapse agent navigation" : "Expand agent navigation"} title={expanded ? "Collapse agent navigation" : "Expand agent navigation"} className="flex h-10 w-full cursor-pointer items-center gap-3 rounded-lg px-3 text-zinc-400 hover:bg-zinc-800 hover:text-white"><span aria-hidden="true">{expanded ? "«" : "»"}</span>{expanded && <span className="text-sm">Collapse</span>}</button>
        <div className="border-t border-zinc-800 pt-2">{link(sections[sections.length - 1]!)}</div>
      </div>
    </nav>
    <div className="flex min-h-[calc(100dvh-5rem)] w-full min-w-0 flex-col">
      <Outlet />
    </div>
  </div>;
}

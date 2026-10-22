import { Link, Outlet } from "@tanstack/react-router";
import { useState } from "react";
import { AgentLifecycleSubscriber } from "./agent-lifecycle-subscriber.js";
import { AgentEventsProvider } from "./agent-events.js";
import { useIndicatorCatalogEvents } from "./use-indicator-catalog-events.js";
import type { AgentRow } from "../db/agent-schema.js";

const sections = [
  { label: "Dashboard", to: "/agents/$agentId", icon: "M3 3h6v6H3zM13 3h6v6h-6zM3 13h6v6H3zM13 13h6v6h-6z" },
  { label: "Charts", to: "/agents/$agentId/chart", icon: "M3 3v18h18M6 15l5-6 4 3 5-7" },
  { label: "Transactions", to: "/agents/$agentId/transactions", icon: "M3 7h17m-4-4 4 4-4 4M21 17H4m4-4-4 4 4 4" },
  { label: "Chat", to: "/agents/$agentId/chat", icon: "M4 4h16v12H9l-5 4z" },
  { label: "Sub-Agents", to: "/agents/$agentId/sub-agents", icon: "M9 3h6v6H9zM3 16h6v6H3zM15 16h6v6h-6zM12 9v4M6 16v-3h12v3" },
  { label: "Notifications", to: "/agents/$agentId/notifications", icon: "M6 8a6 6 0 0 1 12 0c0 7 3 7 3 9H3c0-2 3-2 3-9M9 21h6" },
  { label: "Settings", to: "/agents/$agentId/settings", icon: "M4 6h16M4 12h16M4 18h16M8 3v6M16 9v6M10 15v6" },
] as const;

const chartSections = [
  { label: "Indicators", to: "/agents/$agentId/indicators", icon: "M3 12h18M3 18l5-12 5 10 4-8 4 10" },
] as const;

const memorySections = [
  { label: "Insights", to: "/agents/$agentId/memories", search: { scope: "" }, icon: "M4 4h16v16H4zM8 8h8M8 12h8M8 16h5" },
] as const;

function IndicatorCatalogSubscriber({ agentId }: { agentId: string }) {
  useIndicatorCatalogEvents(agentId);
  return null;
}

export function AgentLayout({ agent, initialExpanded }: { agent: AgentRow; initialExpanded: boolean }) {
  const [expanded, setExpanded] = useState(initialExpanded);
  const [memoriesExpanded, setMemoriesExpanded] = useState(true);
  const [chartsExpanded, setChartsExpanded] = useState(true);
  function toggle() {
    setExpanded((value) => {
      document.cookie = `hv-agent-nav-expanded=${!value}; Max-Age=31536000; Path=/; SameSite=Strict`;
      return !value;
    });
  }
  const link = (section: typeof sections[number] | typeof memorySections[number] | typeof chartSections[number]) => <Link key={section.label} to={section.to} params={{ agentId: agent.id }} search={"search" in section ? section.search : undefined} activeOptions={{ exact: true }} title={section.label} aria-label={section.label}
    onClick={section.label === "Charts" ? () => setChartsExpanded(true) : undefined}
    aria-expanded={section.label === "Charts" ? chartsExpanded : undefined}
    aria-controls={section.label === "Charts" ? "agent-chart-sections" : undefined}
    className="flex h-10 cursor-pointer items-center gap-3 rounded-lg px-3 text-zinc-400 hover:bg-zinc-800 hover:text-white"
    activeProps={{ className: "bg-zinc-800 text-white", "aria-current": "page" }}>
    <svg className="h-5 w-5 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={section.icon} /></svg>
    {expanded && <><span className="text-sm">{section.label}</span>{section.label === "Charts" && <span className="ml-auto text-xs" aria-hidden="true">{chartsExpanded ? "▾" : "▸"}</span>}</>}
  </Link>;
  return <div className={expanded ? "pl-48" : "pl-14 sm:pl-16"}>
    <nav aria-label="Agent sections" className={`fixed bottom-0 left-0 top-14 z-30 flex flex-col border-r border-zinc-800 bg-[#111214] p-1.5 ${expanded ? "w-48" : "w-14 sm:w-16"}`}>
      <div className="flex min-h-0 flex-col gap-1 overflow-y-auto">
        {link(sections[0])}
        {link(sections[1])}
        <div id="agent-chart-sections" hidden={!chartsExpanded} className={expanded ? "ml-4 space-y-1 border-l border-zinc-800 pl-1" : "space-y-1"}>{chartsExpanded && chartSections.map(link)}</div>
        {link(sections[2])}
        <hr className="my-2 border-zinc-800" />
        {sections.slice(3, -2).map(link)}
        <Link to="/agents/$agentId/memories" params={{ agentId: agent.id }} search={{}} onClick={() => setMemoriesExpanded(true)} activeOptions={{ exact: true, includeSearch: true }} activeProps={{ className: "bg-zinc-800 text-white", "aria-current": "page" }} aria-expanded={memoriesExpanded} aria-controls="agent-memory-sections" aria-label="Memories" title="Memories" className="flex h-10 shrink-0 cursor-pointer items-center gap-3 rounded-lg px-3 text-zinc-400 hover:bg-zinc-800 hover:text-white">
          <svg className="h-5 w-5 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M4 4h16v16H4zM8 8h8M8 12h8M8 16h5" /></svg>
          {expanded && <><span className="text-sm">Memories</span><span className="ml-auto text-xs" aria-hidden="true">{memoriesExpanded ? "▾" : "▸"}</span></>}
        </Link>
        <div id="agent-memory-sections" hidden={!memoriesExpanded} className={expanded ? "ml-4 space-y-1 border-l border-zinc-800 pl-1" : "space-y-1"}>{memoriesExpanded && memorySections.map(link)}</div>
      </div>
      <div className="mt-auto space-y-2">
        <button type="button" onClick={toggle} aria-expanded={expanded} aria-label={expanded ? "Collapse agent navigation" : "Expand agent navigation"} title={expanded ? "Collapse agent navigation" : "Expand agent navigation"} className="flex h-10 w-full cursor-pointer items-center gap-3 rounded-lg px-3 text-zinc-400 hover:bg-zinc-800 hover:text-white"><span className="flex h-5 w-5 shrink-0 items-center justify-center" aria-hidden="true">{expanded ? "«" : "»"}</span>{expanded && <span className="text-sm">Collapse</span>}</button>
        <div className="space-y-1 border-t border-zinc-800 pt-2">{sections.slice(-2).map(link)}</div>
      </div>
    </nav>
    <div className="flex min-h-[calc(100dvh-5rem)] w-full min-w-0 flex-col">
      <AgentEventsProvider agentId={agent.id}>
        <AgentLifecycleSubscriber agentId={agent.id} />
        <IndicatorCatalogSubscriber agentId={agent.id} />
        <Outlet />
      </AgentEventsProvider>
    </div>
  </div>;
}

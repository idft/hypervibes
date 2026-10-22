import { Link } from "@tanstack/react-router";
import { useEffect, useRef } from "react";

export interface SwitcherAgent { id: string; name: string; enabled: boolean; tradingEnabled: boolean }

function AgentStatusDot({ agentEnabled, tradingEnabled }: { agentEnabled: boolean; tradingEnabled: boolean }) {
  const enabled = agentEnabled && tradingEnabled;
  const label = !agentEnabled ? "Agent disabled" : tradingEnabled ? "Trading enabled" : "Trading disabled";
  return <span role="img" aria-label={label} title={label} className={`h-2 w-2 shrink-0 rounded-full ${enabled ? "bg-emerald-400" : "bg-amber-400"}`} />;
}

/** The legacy selector's native disclosure behavior, with client-side navigation. */
export function AgentSwitcher({ agents, selectedAgentId }: { agents: SwitcherAgent[]; selectedAgentId?: string }) {
  const details = useRef<HTMLDetailsElement>(null);
  const summary = useRef<HTMLElement>(null);
  const selected = agents.find((agent) => agent.id === selectedAgentId);
  function close() { if (details.current) details.current.open = false; }
  useEffect(() => { close(); }, [selectedAgentId]);
  useEffect(() => {
    const outside = (event: MouseEvent) => { if (!details.current?.contains(event.target as Node)) close(); };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && details.current?.open) { close(); summary.current?.focus(); }
    };
    document.addEventListener("click", outside);
    document.addEventListener("keydown", escape);
    return () => { document.removeEventListener("click", outside); document.removeEventListener("keydown", escape); };
  }, []);
  return <details ref={details} className="relative h-full min-w-0 max-w-[45vw] sm:max-w-64" data-agent-selector>
    <summary ref={summary} aria-label="Switch Agent" className="flex h-full cursor-pointer list-none items-center gap-2 px-4 text-sm text-zinc-300 hover:bg-zinc-800 hover:text-white [&::-webkit-details-marker]:hidden">
      {selected && <AgentStatusDot agentEnabled={selected.enabled} tradingEnabled={selected.tradingEnabled} />}
      <span className="truncate">{selected?.name ?? "Agents"}</span>
      <svg className="h-4 w-4 shrink-0 text-zinc-500" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true"><path fillRule="evenodd" clipRule="evenodd" d="M5.23 7.21a.75.75 0 0 1 1.06.02L10 11.17l3.71-3.94a.75.75 0 1 1 1.09 1.04l-4.25 4.5a.75.75 0 0 1-1.09 0l-4.25-4.5a.75.75 0 0 1 .02-1.06Z" /></svg>
    </summary>
    <div className="absolute left-0 top-full mt-2 max-h-[calc(100dvh-5rem)] w-64 overflow-y-auto rounded-lg border border-zinc-700 bg-[#1b1d20] p-1 shadow-2xl">
      {agents.map((agent) => <Link key={agent.id} to="/agents/$agentId" params={{ agentId: agent.id }} onClick={close} aria-current={agent.id === selectedAgentId ? "page" : undefined} className="flex cursor-pointer items-center gap-2 rounded-md px-3 py-2 text-sm text-zinc-300 hover:bg-zinc-800 hover:text-white"><AgentStatusDot agentEnabled={agent.enabled} tradingEnabled={agent.tradingEnabled} /><span className="truncate">{agent.name}</span></Link>)}
      {agents.length > 0 && <div className="my-1 border-t border-zinc-700" />}
      <Link to="/agents/new" onClick={close} className="block cursor-pointer rounded-md px-3 py-2 text-sm font-medium text-sky-300 hover:bg-zinc-800 hover:text-sky-200">Create new agent</Link>
    </div>
  </details>;
}

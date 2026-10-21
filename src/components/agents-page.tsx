import type { AgentRow } from "../db/agent-schema.js";
import { SetupChecklist, type SetupState } from "./setup-checklist.js";

export function AgentsPage({ agents, setup }: { agents: AgentRow[]; setup?: SetupState }) {
  return (
    <div className="mx-auto max-w-[1400px]">
      <header className="flex items-center justify-between gap-4 border-b border-zinc-800 pb-6">
        <h1 className="text-3xl font-semibold tracking-tight text-white">Agents</h1>
        <a href="/agents/new" className="cursor-pointer rounded-full bg-violet-600 px-5 py-2 text-sm font-medium text-white hover:bg-violet-500">Create a new agent</a>
      </header>
      <div className="mt-6" />
      {agents.length === 0 && setup && <div className="mb-6"><SetupChecklist state={setup} agentCount={0} /></div>}
      {agents.length === 0 ? <section className="flex min-h-48 items-center justify-center rounded-xl border border-dashed border-zinc-800"><h2 className="text-xl text-white">No agents yet</h2></section> :
        <ul className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">{agents.map((agent) => <li key={agent.id}>
          <a href={`/agents/${agent.id}`} className="block cursor-pointer rounded-xl border border-zinc-800 bg-[#0d0d10] p-5 hover:border-violet-700">
            <h2 className="text-xl font-semibold text-white">{agent.name}</h2>
            {agent.state !== "ready" && <p className="mt-2 text-sm text-zinc-400">{agent.state === "failed" ? "Initialization failed" : agent.state === "stopping" || agent.state === "stopped" ? "Deletion pending — retry in Settings" : "Initializing…"}</p>}
            {agent.tradingAccountAddress
              ? <p className="mt-1 break-all font-mono text-xs text-zinc-500">{agent.tradingAccountAddress}</p>
              : <p className="mt-1 text-xs text-zinc-500">Chat only</p>}
          </a>
        </li>)}</ul>}
    </div>
  );
}
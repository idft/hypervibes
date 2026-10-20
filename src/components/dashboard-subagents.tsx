import type { DashboardSubagent } from "../agents/subagent-dashboard.js";
import { RelativeTime, RunStatus, runUrl } from "./subagent-run-history.js";

export function DashboardSubagents({ agentId, subagents, error }: { agentId: string; subagents: DashboardSubagent[]; error?: string }) {
  const base = `/agents/${encodeURIComponent(agentId)}/sub-agents`;
  return <section aria-label="Sub-agents" className="rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
    <header className="mb-4 flex flex-wrap items-center justify-between gap-3">
      <h3 className="text-lg font-medium text-white">Sub-agents</h3>
      <a href={`${base}/new`} className="cursor-pointer text-sm text-violet-400 hover:text-violet-300 hover:underline">Create a new sub-agent</a>
    </header>
    {error ? <p role="alert" className="text-sm text-amber-200">Sub-agents unavailable: {error}</p> : !subagents.length ? <p className="text-sm text-zinc-500">No sub-agents yet. Create a new sub-agent to get started.</p> : <ul className="space-y-4">
      {subagents.map((subagent) => <li key={subagent.id} className="overflow-hidden rounded-xl border border-zinc-800">
        <h4><a href={`${base}/${encodeURIComponent(subagent.id)}`} className="flex cursor-pointer items-center justify-between gap-3 bg-zinc-900/50 px-4 py-3 font-medium text-zinc-100 hover:bg-zinc-800/60 hover:text-violet-300">
          {subagent.name}{!subagent.enabled && <span className="text-xs font-normal text-zinc-500">Disabled</span>}
        </a></h4>
        {!subagent.runs.length ? <p className="px-4 py-3 text-sm text-zinc-500">No runs yet.</p> : <ul className="divide-y divide-zinc-800/60">{subagent.runs.map((run) => <li key={run.id}>
          <a href={runUrl(agentId, subagent.id, run.id)} className="flex cursor-pointer items-center gap-3 px-4 py-3 text-sm hover:bg-zinc-900 focus-visible:outline-violet-400">
            <RunStatus status={run.status} />
            <span className="min-w-0 flex-1 truncate text-zinc-300">{run.result || (run.status === "running" ? "Running…" : run.status === "queued" ? "Waiting to run…" : "No result message.")}</span>
            <span className="shrink-0 text-xs text-zinc-500"><RelativeTime timestamp={run.createdAt} /></span>
          </a>
        </li>)}</ul>}
      </li>)}
    </ul>}
  </section>;
}

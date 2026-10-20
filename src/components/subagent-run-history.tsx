import { useEffect, useState } from "react";
import { format } from "timeago.js";
import type { SubagentRun } from "../db/subagent-schema.js";
import type { RunPagination, RunSummary } from "../agents/subagent-history.js";

const states = {
  completed: { color: "text-emerald-400", path: "m5 12 4 4L19 6" },
  failed: { color: "text-red-400", path: "m7 7 10 10M17 7 7 17" },
  running: { color: "text-violet-400 animate-spin", path: "M12 3a9 9 0 1 1-9 9" },
  queued: { color: "text-amber-300", path: "M12 8v4l3 2M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0" },
  cancelled: { color: "text-zinc-400", path: "m6 6 12 12M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0" },
  skipped: { color: "text-zinc-500", path: "m5 5 10 7-10 7zM19 5v14" },
};
export function RunStatus({ status }: { status: SubagentRun["status"] }) {
  const state = states[status];
  return <span role="img" aria-label={status} title={status} className={`inline-flex align-middle ${state.color}`}><svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" className="size-5"><path d={state.path} /></svg></span>;
}
export function RelativeTime({ timestamp }: { timestamp: number }) {
  const [, tick] = useState(0);
  useEffect(() => { const timer = setInterval(() => tick((value) => value + 1), 30000); return () => clearInterval(timer); }, []);
  const exact = new Date(timestamp).toISOString();
  return <time dateTime={exact} title={exact} suppressHydrationWarning>{format(timestamp)}</time>;
}
export function runUrl(agentId: string, subagentId: string, runId: string) {
  return `/agents/${encodeURIComponent(agentId)}/sub-agents/${encodeURIComponent(subagentId)}/runs/${encodeURIComponent(runId)}`;
}
export function SubagentRunHistory({ agentId, runs, pending, onStop, pagination, loading = false, onPage }: { agentId: string; runs: RunSummary[]; pending: boolean; onStop: (id: string) => void; pagination?: RunPagination; loading?: boolean; onPage?: (page: number) => void }) {
  if (!runs.length) return <p className="text-sm text-zinc-500">No assignments yet.</p>;
  return <div aria-busy={loading} className="space-y-4"><div className="overflow-x-auto"><table className="w-full min-w-[640px] table-fixed text-left text-sm text-zinc-300">
    <caption className="sr-only">Recent sub-agent assignments</caption>
    <thead className="border-b border-zinc-800 text-xs text-zinc-500"><tr><th className="w-36 px-3 py-3">Created</th><th className="w-28 px-3 py-3">Source</th><th className="px-3 py-3">Instruction</th><th className="w-16 px-3 py-3">Status</th><th className="w-16 px-3 py-3"><span className="sr-only">Actions</span></th></tr></thead>
    <tbody>{runs.map((run) => <tr key={run.id} className="cursor-pointer border-b border-zinc-800/60 hover:bg-zinc-900 focus-within:bg-zinc-900" onClick={(event) => { if (!(event.target as HTMLElement).closest("a, button")) event.currentTarget.querySelector("a")?.click(); }}>
      <td className="px-3 py-3 text-xs text-zinc-400"><RelativeTime timestamp={run.createdAt} /></td>
      <td className="px-3 py-3">{run.source === "schedule" ? "Scheduled" : run.source === "delegation" ? "Delegated" : run.source === "event" ? "Event" : "Manual"}</td>
      <td className="px-3 py-3"><a href={runUrl(agentId, run.subagentId, run.id)} className="block truncate text-zinc-200 hover:text-violet-300 focus-visible:outline-violet-400" title={run.task}>{run.task}</a></td>
      <td className="px-3 py-3"><RunStatus status={run.status} /></td>
      <td className="px-3 py-3">{(run.status === "queued" || run.status === "running") && <button disabled={pending} aria-label="Stop assignment" onClick={() => onStop(run.id)} className="cursor-pointer text-xs text-red-300 hover:underline disabled:cursor-wait disabled:opacity-50">Stop</button>}</td>
    </tr>)}</tbody>
  </table></div>
    {pagination && pagination.pages > 1 && <nav aria-label="Run history pagination" className="flex flex-wrap items-center justify-between gap-3 text-sm text-zinc-400">
      <p aria-live="polite">Page {pagination.page} of {pagination.pages} · {pagination.total} runs</p>
      <div className="flex gap-2"><button disabled={loading || pending || pagination.page <= 1} onClick={() => onPage?.(pagination.page - 1)} className="cursor-pointer rounded-lg border border-zinc-700 px-3 py-2 hover:bg-zinc-900 disabled:cursor-not-allowed disabled:opacity-40">Previous</button><button disabled={loading || pending || pagination.page >= pagination.pages} onClick={() => onPage?.(pagination.page + 1)} className="cursor-pointer rounded-lg border border-zinc-700 px-3 py-2 hover:bg-zinc-900 disabled:cursor-not-allowed disabled:opacity-40">Next</button></div>
    </nav>}
  </div>;
}

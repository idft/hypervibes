import { useEffect, useRef, useState } from "react";
import { useRouter } from "@tanstack/react-router";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { subagentMutations } from "../web/subagent-mutations.js";
import { toast } from "sonner";
import { format } from "timeago.js";
import { SubagentEnabledToggle } from "./subagent-enabled-toggle.js";
import { RunSubagentDialog } from "./run-subagent-dialog.js";
import type { DashboardSubagent } from "../agents/subagent-dashboard.js";
import { RelativeTime, RunStatus, runUrl } from "./subagent-run-history.js";

export function DashboardSubagents({ agentId, subagents, error }: { agentId: string; subagents: DashboardSubagent[]; error?: string }) {
  const [runTarget, setRunTarget] = useState<string | null>(null);
  const [workers, setWorkers] = useState(subagents);
  const [refreshError, setRefreshError] = useState("");
  const [loadError, setLoadError] = useState(error);
  useEffect(() => { setWorkers(subagents); setLoadError(error); }, [subagents, error]);
  useEffect(() => {
    setRefreshError("");
    const source = new EventSource(`/api/agents/${encodeURIComponent(agentId)}/sub-agents/events`);
    source.addEventListener("subagents", (event) => {
      setWorkers(JSON.parse((event as MessageEvent).data) as DashboardSubagent[]);
      setRefreshError(""); setLoadError(undefined);
    });
    const interrupted = () => setRefreshError("Live sub-agent updates interrupted. Reconnecting… Activity may be out of date.");
    source.onerror = interrupted;
    source.addEventListener("unavailable", interrupted);
    source.addEventListener("shutdown", interrupted);
    source.addEventListener("unauthorized", () => { source.close(); window.location.assign("/login"); });
    source.addEventListener("stream-error", () => { source.close(); setRefreshError("Sub-agent activity is too large to stream. Reload the page."); });
    return () => source.close();
  }, [agentId]);
  const base = `/agents/${encodeURIComponent(agentId)}/sub-agents`;
  return <section aria-label="Sub-agents" className="rounded-2xl border border-zinc-800 bg-[#0d0d10] p-4">
    <header className="mb-3">
      <h3 className="text-lg font-medium text-white">Sub-agents</h3>
    </header>
    {loadError ? <p role="alert" className="text-sm text-amber-200">Sub-agents unavailable: {loadError}</p> : !workers.length ? <p className="text-sm text-zinc-500">No sub-agents yet. <a href={`${base}/new`} className="cursor-pointer text-violet-400 hover:text-violet-300 hover:underline">Create a new sub-agent</a> to get started.</p> : <ul className="space-y-2">
      {workers.map((subagent) => <li key={subagent.id} className="overflow-hidden rounded-xl border border-zinc-800">
        <div className="flex flex-wrap items-center gap-2 bg-zinc-900/50 px-3 py-2">
          <h4 className="min-w-0"><a href={`${base}/${encodeURIComponent(subagent.id)}`} className="cursor-pointer font-medium text-zinc-100 hover:text-violet-300">{subagent.name}</a></h4>
          <button type="button" aria-label={`Run now: ${subagent.name}`} title="Run now" aria-haspopup="dialog" onClick={() => setRunTarget(subagent.id)} className="inline-flex shrink-0 cursor-pointer items-center justify-center rounded-full p-1.5 text-violet-300 hover:bg-violet-950"><svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" className="size-4"><path d="m13 2-9 12h7l-1 8 10-12h-7z" /></svg></button>
          <DashboardWorkerControls agentId={agentId} definition={subagent} />
        </div>
        {!subagent.runs.length ? <p className="px-3 py-2 text-sm text-zinc-500">No runs yet.</p> : <ul className="divide-y divide-zinc-800/60">{subagent.runs.slice(0, 1).map((run) => <li key={run.id}>
          <a href={runUrl(agentId, subagent.id, run.id)} className="flex cursor-pointer items-center gap-3 px-3 py-2 text-sm hover:bg-zinc-900 focus-visible:outline-violet-400">
            <RunStatus status={run.status} />
            <span className="min-w-0 flex-1 truncate text-zinc-300">{run.result || (run.status === "running" ? "Running…" : run.status === "queued" ? "Waiting to run…" : "No result message.")}</span>
            <span className="shrink-0 text-xs text-zinc-500"><RelativeTime timestamp={run.createdAt} /></span>
          </a>
        </li>)}</ul>}
      </li>)}
    </ul>}
    {refreshError && <p role="alert" className="mt-3 text-sm text-amber-200">{refreshError}</p>}
    {runTarget && <DashboardRunDialog key={`${agentId}:${runTarget}`} agentId={agentId} subagentId={runTarget} onClose={() => setRunTarget(null)} />}
  </section>;
}

function NextRun({ timestamp }: { timestamp: number }) {
  const [, tick] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => tick((value) => value + 1), 1000);
    return () => clearInterval(timer);
  }, []);
  const exact = new Date(timestamp).toISOString();
  return <span className="text-xs text-zinc-400">Next run: <time dateTime={exact} title={exact} suppressHydrationWarning>{timestamp > Date.now() ? format(timestamp) : "due now"}</time></span>;
}

function DashboardWorkerControls({ agentId, definition }: { agentId: string; definition: DashboardSubagent }) {
  const [settings, setSettings] = useState(definition);
  const client = useQueryClient();
  const mutation = useMutation({
    mutationFn: subagentMutations(client, agentId).saveSection<"enabled">,
    onSuccess: (row) => {
      setSettings((current) => ({ ...current, enabled: row.enabled, schedule: row.schedule, nextRunAt: row.nextRunAt }));
      toast.success(row.enabled ? "Sub-agent enabled" : "Sub-agent paused");
    },
  });
  const pending = mutation.isPending, error = mutation.error?.message;
  useEffect(() => { setSettings(definition); }, [definition]);
  return <div className="flex flex-1 flex-wrap items-center gap-2">
    {settings.enabled && settings.schedule && settings.nextRunAt !== null && <NextRun timestamp={settings.nextRunAt} />}
    <div className="ml-auto flex items-center gap-2">
      <SubagentEnabledToggle enabled={settings.enabled} disabled={pending} onClick={() => mutation.mutate({ id: definition.id, section: "enabled", settings: { enabled: !settings.enabled } })} />
      {error && <p role="alert" className="text-xs text-red-300">{error}</p>}
    </div>
  </div>;
}

function DashboardRunDialog({ agentId, subagentId, onClose }: { agentId: string; subagentId: string; onClose: () => void }) {
  const router = useRouter();
  const [task, setTask] = useState("");
  const assignmentId = useRef<{ task: string; id: string } | null>(null);
  const client = useQueryClient();
  const mutation = useMutation({
    mutationFn: async () => {
      if (assignmentId.current?.task !== task) assignmentId.current = { task, id: crypto.randomUUID() };
      try { return await subagentMutations(client, agentId).run({ id: subagentId, task, requestId: assignmentId.current.id }); }
      catch (cause) { throw new Error(`${cause instanceof Error ? cause.message : "Unable to confirm the assignment."} Retry with the same task to reuse its request ID.`, { cause }); }
    },
    onSuccess: async (run) => {
      toast.success("Assignment started");
      await router.navigate({ to: "/agents/$agentId/sub-agents/$subagentId/runs/$runId", params: { agentId, subagentId, runId: run.id } });
      onClose();
    },
  });
  async function startRun() { await mutation.mutateAsync(); }
  return <RunSubagentDialog task={task} onTaskChange={setTask} onCancel={onClose} onRun={startRun} />;
}

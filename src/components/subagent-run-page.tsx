import { useRef, useState } from "react";
import type { EntryRecord } from "@earendil-works/pi-durable";
import type { SubagentService } from "../agents/subagent-service.js";
import { workerInstructions } from "../agents/subagent-input.js";
import { getSubagentRun, stopSubagentRun } from "../web/subagents.js";
import { RelativeTime, RunStatus } from "./subagent-run-history.js";

const panel = "space-y-4 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6";
const pre = "whitespace-pre-wrap break-words text-sm text-zinc-300";
const button = "cursor-pointer rounded-full border border-zinc-700 px-4 py-2 text-sm text-zinc-300 hover:bg-zinc-800 disabled:cursor-wait disabled:opacity-50";
type RunDetail = Awaited<ReturnType<SubagentService["runDetail"]>>;
function Transcript({ entries }: { entries: EntryRecord[] }) {
  return <div className="space-y-4">{entries.map((entry) => <article key={entry.id} className="space-y-3 rounded-xl border border-zinc-800 p-4">
    {(entry.model ?? []).map((message, index) => <div key={index} className="space-y-2">
      <h3 className="text-xs font-semibold uppercase tracking-wide text-zinc-400">{message.role === "toolResult" ? `Tool result · ${message.toolName}${message.isError ? " · Error" : ""}` : message.role}</h3>
      {typeof message.content === "string" ? <pre className={pre}>{message.content}</pre> : message.content.map((part, partIndex) => <div key={partIndex}>
        {part.type === "text" ? <pre className={pre}>{part.text}</pre> : part.type === "thinking" ? <details open><summary className="cursor-pointer text-xs text-zinc-400">Thinking</summary><pre className={pre}>{part.thinking}</pre></details> : part.type === "toolCall" ? <div className="space-y-2 rounded-lg bg-zinc-900 p-3"><h4 className="text-sm text-violet-300">Tool call · {part.name}</h4><pre className={pre}>{JSON.stringify(part.arguments, null, 2)}</pre></div> : <pre className={pre}>{JSON.stringify(part, null, 2)}</pre>}
      </div>)}
      {message.role === "assistant" && message.errorMessage && <p className="text-sm text-red-300">{message.errorMessage}</p>}
    </div>)}
    <details><summary className="cursor-pointer text-xs text-zinc-500">Raw transcript entry · {entry.kind}</summary><pre className={`mt-3 ${pre}`}>{JSON.stringify(entry, null, 2)}</pre></details>
  </article>)}</div>;
}
export function SubagentRunPage({ initial }: { initial: RunDetail }) {
  const [detail, setDetail] = useState(initial), [pending, setPending] = useState(false), [error, setError] = useState("");
  const inFlight = useRef(false);
  const { run, entries, prior } = detail;
  async function refresh(stop = false) {
    if (inFlight.current) return;
    inFlight.current = true; setPending(true); setError("");
    let stopped = false;
    try {
      if (stop) {
        const result = await stopSubagentRun({ data: { agentId: run.agentId, runId: run.id } });
        if (!result.ok) throw new Error(result.error);
        stopped = true;
      }
      const result = await getSubagentRun({ data: { agentId: run.agentId, id: run.subagentId, runId: run.id } });
      if (!result.ok) throw new Error(result.error);
      setDetail(result.data);
    } catch (cause) { setError(`${stopped ? "Assignment stopped, but refresh failed. " : ""}${cause instanceof Error ? cause.message : "Unable to refresh assignment. Please retry."}`); }
    finally { inFlight.current = false; setPending(false); }
  }
  return <div className="w-full min-w-0 space-y-3">
    <header className={panel}>
      <a href={`/agents/${encodeURIComponent(run.agentId)}/sub-agents/${encodeURIComponent(run.subagentId)}`} className="text-sm text-violet-300 hover:underline">← {detail.definition.name}</a>
      <div className="flex flex-wrap items-center justify-between gap-3"><h2 className="flex items-center gap-3 text-lg font-medium text-white"><RunStatus status={run.status} />Run detail · {run.status}</h2><div className="flex gap-2"><button disabled={pending} onClick={() => { void refresh(); }} className={button}>Refresh</button>{(run.status === "queued" || run.status === "running") && <button disabled={pending} onClick={() => { void refresh(true); }} className={`${button} text-red-300`}>Stop assignment</button>}</div></div>
      <p className="text-sm text-zinc-400">{run.source} · Created <RelativeTime timestamp={run.createdAt} />{run.finishedAt !== null && <> · Finished <RelativeTime timestamp={run.finishedAt} /></>}</p>
      <p className="text-xs text-zinc-500">{run.config.provider}/{run.config.modelId}{run.config.thinkingLevel && ` · Thinking: ${run.config.thinkingLevel}`} · {run.config.persistent ? "Persistent context" : "Fresh context"}</p>
      <p className="text-xs text-zinc-500">Stopping an assignment does not cancel existing exchange orders.</p>
      {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
    </header>
    <section className={panel}><h2 className="font-medium text-white">Instructions at execution</h2><pre className={pre}>{workerInstructions(run.config)}</pre><p className="text-xs text-zinc-500">Allowed tools: {run.config.tools.join(", ") || "None"}</p><details><summary className="cursor-pointer text-sm text-zinc-400">Saved configuration</summary><pre className={`mt-3 ${pre}`}>{JSON.stringify(run.config, null, 2)}</pre></details></section>
    <section className={panel}><h2 className="font-medium text-white">Assignment</h2><pre className={pre}>{run.task}</pre></section>
    {prior.length > 0 && <section className={panel}><details><summary className="cursor-pointer font-medium text-white">Earlier conversation context · {prior.length} entries</summary><p className="my-4 text-sm text-zinc-400">History preceding this assignment in the persistent conversation. Later assignments are not shown.</p><Transcript entries={prior} /></details></section>}
    <section className={panel}><h2 className="font-medium text-white">Conversation transcript</h2>{entries.length ? <Transcript entries={entries} /> : <p className="text-sm text-zinc-500">No transcript entries yet.</p>}</section>
    <section className={panel}><h2 className="font-medium text-white">Final result</h2><pre className={pre}>{run.result || (run.status === "queued" || run.status === "running" ? "Assignment has not finished yet. Refresh for updates." : "No final result was recorded.")}</pre></section>
  </div>;
}

import { useRef, useState } from "react";
import type { EntryRecord } from "@earendil-works/pi-durable";
import type { RunDetail } from "../agents/subagent-service.js";
import { workerInstructions } from "../agents/subagent-input.js";
import { stopSubagentRun } from "../web/subagents.js";
import { useSubagentRun } from "./use-subagent-run.js";
import { RelativeTime, RunStatus } from "./subagent-run-history.js";
import { ToolCall } from "./tool-activity.js";
import { toolActivity, type ToolActivity } from "../research/tool-activity.js";
import { ProviderLogo } from "./provider-logo.js";
import { ChatBubble } from "./chat-bubble.js";
import { ArtifactContent } from "./artifact-content.js";

const panel = "space-y-4 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6";
const pre = "whitespace-pre-wrap break-words text-sm text-zinc-300";
const button = "cursor-pointer rounded-full border border-zinc-700 px-4 py-2 text-sm text-zinc-300 hover:bg-zinc-800 disabled:cursor-wait disabled:opacity-50";
const breadcrumbLink = "cursor-pointer text-sm text-zinc-400 hover:text-white";
function Transcript({ entries, agentId, tools = [] }: { entries: EntryRecord[]; agentId: string; tools?: ToolActivity[] }) {
  const activity = new Map(entries.flatMap((entry) => toolActivity([entry])).map((tool) => [tool.id, tool]));
  for (const tool of tools) activity.set(tool.id, tool);
  for (const entry of entries) for (const message of entry.model ?? []) {
    if (message.role !== "assistant") continue;
    for (const part of message.content) if (part.type === "toolCall") {
      const tool = activity.get(part.id);
      activity.set(part.id, { id: part.id, name: part.name, status: "pending", ...tool, arguments: tool?.arguments ?? JSON.stringify(part.arguments).slice(0, 500) });
    }
  }
  const callIds = new Set(entries.flatMap((entry) => (entry.model ?? []).flatMap((message) => message.role === "assistant" ? message.content.flatMap((part) => part.type === "toolCall" ? [part.id] : []) : [])));
  function renderTool(id: string) {
    return [...activity.values()].filter((tool) => tool.id === id || tool.id.startsWith(`${id}:`)).map((tool) => <ToolCall key={tool.id} tool={tool} />);
  }
  const resultIds = new Set(entries.flatMap((entry) => (entry.model ?? []).flatMap((message) => message.role === "toolResult" ? [message.toolCallId] : [])));
  type TranscriptMessage = { id: string; message: NonNullable<EntryRecord["model"]>[number] };
  const groups: { id: string; role: TranscriptMessage["message"]["role"]; messages: TranscriptMessage[] }[] = [];
  for (const entry of entries) for (const [index, message] of (entry.model ?? []).entries()) {
    if (message.role === "toolResult" && callIds.has(message.toolCallId)) continue;
    const item = { id: `${entry.id}:${index}`, message };
    const previous = groups.at(-1);
    if (message.role === "assistant" && previous?.role === "assistant") previous.messages.push(item);
    else groups.push({ id: item.id, role: message.role, messages: [item] });
  }
  return <div className="space-y-4">{groups.map((group) => <ChatBubble key={group.id} role={group.role === "toolResult" ? "tool" : group.role}>
    {(group.role === "assistant" || group.role === "user") && <h3 className="text-xs font-semibold uppercase tracking-wide text-zinc-400">{group.role}</h3>}
    {group.messages.map(({ message, id }) => message.role === "toolResult" ? <div key={id}>{renderTool(message.toolCallId)}</div> : message.role === "system" ? <details key={id}>
      <summary className="cursor-pointer text-xs font-semibold uppercase tracking-wide text-zinc-400">System</summary>
      <div className="mt-3 space-y-3">
        <pre className={pre}>{typeof message.content === "string" ? message.content : message.content.map((part) => part.text).join("\n")}</pre>
        {Object.entries(message.sections ?? {}).map(([name, content]) => <div key={name} className="space-y-2"><h4 className="text-xs font-medium text-zinc-400">{name}</h4><pre className={pre}>{content ?? "Section removed"}</pre></div>)}
        {!!message.toolsAdded?.length && <p className="text-xs text-zinc-400">Tools added: {message.toolsAdded.map((tool) => tool.name).join(", ")}</p>}
        {!!message.toolsRemoved?.length && <p className="text-xs text-zinc-400">Tools removed: {message.toolsRemoved.map((tool) => tool.name).join(", ")}</p>}
      </div>
    </details> : <div key={id} className="space-y-2">
      {typeof message.content === "string" ? <ArtifactContent agentId={agentId} content={message.content} /> : message.content.map((part, partIndex) => <div key={partIndex}>
        {part.type === "text" ? <ArtifactContent agentId={agentId} content={part.text} /> : part.type === "thinking" ? <details open><summary className="cursor-pointer text-xs text-zinc-400">Thinking</summary><pre className={pre}>{part.thinking}</pre></details> : part.type === "toolCall" ? <>{renderTool(part.id)}</> : <pre className={pre}>{JSON.stringify(part, null, 2)}</pre>}
      </div>)}
      {message.role === "assistant" && message.errorMessage && <p className="text-sm text-red-300">{message.errorMessage}</p>}
    </div>)}
  </ChatBubble>)}{[...activity.values()].filter((tool) => ![...callIds, ...resultIds].some((id) => tool.id === id || tool.id.startsWith(`${id}:`))).map((tool) => <ToolCall key={tool.id} tool={tool} />)}</div>;
}
export function SubagentRunPage({ initial }: { initial: RunDetail }) {
  const [pending, setPending] = useState(false), [error, setError] = useState("");
  const inFlight = useRef(false);
  const { detail, connectionError } = useSubagentRun(initial.run.agentId, initial.run.subagentId, initial.run.id, initial);
  const { run } = detail;
  const subagentUrl = `/agents/${encodeURIComponent(run.agentId)}/sub-agents/${encodeURIComponent(run.subagentId)}`;
  async function stop() {
    if (inFlight.current) return;
    inFlight.current = true; setPending(true); setError("");
    try {
      const result = await stopSubagentRun({ data: { agentId: run.agentId, runId: run.id } });
      if (!result.ok) throw new Error(result.error);
      // The live stream replaces any manual refresh here.
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to stop the assignment. Please retry."); }
    finally { inFlight.current = false; setPending(false); }
  }
  return <div className="w-full min-w-0 space-y-3">
    <section className={panel}>
      <header className="flex flex-wrap items-center justify-between gap-3">
        <nav aria-label="Run breadcrumb" className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          <a href={`/agents/${encodeURIComponent(run.agentId)}/sub-agents`} className={breadcrumbLink}><span aria-hidden="true">← </span>Sub-agents</a>
          <span aria-hidden="true" className="text-zinc-600">/</span>
          <a href={subagentUrl} className={breadcrumbLink}>{detail.definition.name}</a>
          <span aria-hidden="true" className="text-zinc-600">/</span>
          <h2 aria-current="page" className="break-words text-lg font-medium text-white">Run {run.id}</h2>
          <RunStatus status={run.status} />
        </nav>
        {(run.status === "queued" || run.status === "running") && <button type="button" disabled={pending} onClick={() => { void stop(); }} className={`${button} shrink-0 text-red-300`}>Stop assignment</button>}
      </header>
      <hr className="border-zinc-800" />
      <p className="text-sm text-zinc-400">{run.source} · Created <RelativeTime timestamp={run.createdAt} />{run.finishedAt !== null && <> · Finished <RelativeTime timestamp={run.finishedAt} /></>}</p>
      <p className="flex items-center gap-1.5 text-xs text-zinc-500"><ProviderLogo provider={run.config.provider} className="h-4 w-4 shrink-0 rounded-sm border border-zinc-800 bg-white p-0.5" /><span>{run.config.provider}/{run.config.modelId}</span><span>{run.config.thinkingLevel && ` · Thinking: ${run.config.thinkingLevel}`} · {run.config.persistent ? "Persistent context" : "Fresh context"}</span></p>
      {connectionError && <p role="status" className="text-sm text-amber-200">{connectionError}</p>}
      {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
      {detail.warnings?.map((warning, index) => <p key={`${warning.code}:${index}`} role="status" className="text-sm text-amber-200">{warning.code.replaceAll("_", " ")}: {warning.message}</p>)}
    </section>
    <section className={panel}><details><summary className="cursor-pointer font-medium text-white">Instructions at execution</summary><div className="mt-4 space-y-4"><pre className={pre}>{detail.context?.instructions ?? workerInstructions(run.config)}</pre><p className="text-xs text-zinc-400">Insights snapshot: {detail.context?.insightStatus ?? "legacy/none"}. Stopping does not cancel exchange orders.</p>{detail.context?.insights && <div className="flex flex-wrap gap-4 text-sm">{([detail.context.insights.agent, detail.context.insights.worker]).map((snapshot) => <a className="cursor-pointer text-violet-300" key={snapshot.scope.kind} href={`/agents/${run.agentId}/memories?scope=${snapshot.scope.kind === "worker" ? encodeURIComponent(snapshot.scope.subagentId) : ""}${snapshot.version ? `&version=${snapshot.version}` : ""}`}>{snapshot.scope.kind === "agent" ? "Agent-wide" : "Worker-specific"} insights v{snapshot.version}</a>)}</div>}{detail.publications?.length ? <div className="space-y-2">{detail.publications.map((publication) => <a key={publication.ref} className="block cursor-pointer text-sm text-violet-300" href={`/agents/${encodeURIComponent(run.agentId)}/memories/${encodeURIComponent(publication.ref)}`}>{publication.ref} · {publication.kind} · {publication.summary}</a>)}</div> : null}<p className="text-xs text-zinc-500">Allowed tools: {run.config.tools.join(", ") || "None"}</p></div></details></section>
    {detail.prior.length > 0 && <section className={panel}><details><summary className="cursor-pointer font-medium text-white">Earlier conversation context · {detail.prior.length} entries</summary><p className="my-4 text-sm text-zinc-400">History preceding this assignment in the persistent conversation. Later assignments are not shown.</p><Transcript entries={detail.prior} agentId={run.agentId} /></details></section>}
    <section className={panel}><h2 className="font-medium text-white">Conversation transcript</h2>{detail.entries.length ? <Transcript entries={detail.entries} agentId={run.agentId} tools={detail.tools} /> : <p className="text-sm text-zinc-500">No transcript entries yet.</p>}</section>
  </div>;
}
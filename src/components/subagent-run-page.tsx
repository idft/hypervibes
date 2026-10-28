import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { subagentMutations } from "../web/subagent-mutations.js";
import type { EntryRecord } from "@earendil-works/pi-durable";
import type { RunDetail } from "../agents/subagent-service.js";
import { workerInstructions } from "../agents/subagent-input.js";
import { useSubagentRun } from "./use-subagent-run.js";
import { RelativeTime, RunStatus } from "./subagent-run-history.js";
import { CodeExecution, ToolCall } from "./tool-activity.js";
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
  function isChild(callId: string, parentId: string) {
    return callId.startsWith(`${parentId}/`) || callId.startsWith(`${parentId}:`);
  }
  function renderTool(id: string) {
    const tool = activity.get(id);
    const calls = [...activity.values()].filter((call) => isChild(call.id, id));
    if (tool?.name === "codemode") return <CodeExecution key={id} tool={tool} calls={calls} />;
    return [tool, ...calls].filter((call): call is ToolActivity => call !== undefined).map((call) => <ToolCall key={call.id} tool={call} />);
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
  </ChatBubble>)}{[...activity.values()].filter((tool) => ![...callIds, ...resultIds].some((id) => tool.id === id || isChild(tool.id, id))).map((tool) => <ToolCall key={tool.id} tool={tool} />)}</div>;
}
function InstructionsDialog({ instructions, children, onClose }: { instructions: string; children: ReactNode; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const element = dialog.current!;
    element.showModal();
    return () => element.close();
  }, []);
  return createPortal(<dialog ref={dialog} aria-labelledby={titleId} onCancel={(event) => { event.preventDefault(); onClose(); }} className="m-auto flex max-h-[calc(100dvh-2rem)] w-[calc(100%_-_2rem)] max-w-3xl flex-col overflow-hidden rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6 text-zinc-200 shadow-2xl backdrop:bg-black/70 backdrop:backdrop-blur-sm">
    <header className="mb-4 flex shrink-0 items-center justify-between gap-3"><h3 id={titleId} className="text-lg font-semibold text-white">Instructions at execution</h3><button type="button" onClick={onClose} className={button}>Close</button></header>
    <pre className={`${pre} min-h-0 overflow-y-auto overscroll-contain`}>{instructions}</pre>
    <footer className="mt-4 shrink-0 space-y-2 border-t border-zinc-800 pt-4">{children}</footer>
  </dialog>, document.body);
}
export function SubagentRunPage({ initial }: { initial: RunDetail }) {
  const client = useQueryClient();
  const mutation = useMutation({ mutationFn: subagentMutations(client, initial.run.agentId).stop });
  const pending = mutation.isPending, error = mutation.error?.message;
  const inFlight = useRef(false);
  async function stop() {
    if (inFlight.current) return;
    inFlight.current = true;
    try { await mutation.mutateAsync(initial.run.id); }
    catch { /* Query retains the cancellation error. */ }
    finally { inFlight.current = false; }
  }
  const [instructionsOpen, setInstructionsOpen] = useState(false);
  const [copyStatus, setCopyStatus] = useState("");
  async function copyRunId() {
    try {
      await navigator.clipboard.writeText(initial.run.id);
      setCopyStatus("Copied");
    } catch {
      setCopyStatus("Unable to copy");
    }
  }
  const { detail, connectionError } = useSubagentRun(initial.run.agentId, initial.run.subagentId, initial.run.id, initial);
  const { run } = detail;
  const transcriptEnd = useRef<HTMLDivElement>(null);
  const transcriptScroll = useRef<HTMLElement>(null);
  const following = useRef(true);
  const wasLive = useRef(false);
  const initializedScroll = useRef(false);
  const [transcriptReady, setTranscriptReady] = useState(false);
  useEffect(() => {
    const scroller = transcriptScroll.current;
    if (!scroller) return;
    const onScroll = () => {
      following.current = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
    };
    scroller.addEventListener("scroll", onScroll, { passive: true });
    return () => scroller.removeEventListener("scroll", onScroll);
  }, []);
  useLayoutEffect(() => {
    const live = run.status === "queued" || run.status === "running";
    // Start every run at the latest activity, and include the final live snapshot.
    if (following.current && (!initializedScroll.current || live || wasLive.current)) {
      transcriptEnd.current?.scrollIntoView({ block: "end", behavior: "instant" });
    }
    if (!initializedScroll.current) {
      initializedScroll.current = true;
      setTranscriptReady(true);
    }
    wasLive.current = live;
  }, [detail.entries, detail.tools, run.status]);
  const subagentUrl = `/agents/${encodeURIComponent(run.agentId)}/sub-agents/${encodeURIComponent(run.subagentId)}`;
  return <div className="flex h-[calc(100dvh-4.5rem)] min-h-0 w-full min-w-0 flex-col gap-3 overflow-hidden sm:h-[calc(100dvh-5rem)]">
    <section aria-label="Run header" className={`${panel} shrink-0`}>
      <header className="flex flex-wrap items-center justify-between gap-3">
        <nav aria-label="Run breadcrumb" className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          <a href={`/agents/${encodeURIComponent(run.agentId)}/sub-agents`} className={breadcrumbLink}><span aria-hidden="true">← </span>Sub-agents</a>
          <span aria-hidden="true" className="text-zinc-600">/</span>
          <a href={subagentUrl} className={breadcrumbLink}>{detail.definition.name}</a>
          <span aria-hidden="true" className="text-zinc-600">/</span>
          <h2 aria-current="page" className="break-words text-lg font-medium text-white">Run {run.id}</h2>
          <button type="button" aria-label="Copy run ID" title="Copy run ID" onClick={() => { void copyRunId(); }} className="shrink-0 cursor-pointer rounded p-1 text-zinc-400 hover:text-white focus-visible:outline-violet-400"><svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5} className="size-4"><rect x="8" y="8" width="12" height="12" rx="2" /><path strokeLinecap="round" strokeLinejoin="round" d="M16 8V4H4v12h4" /></svg></button>
          {copyStatus && <span role="status" className="text-xs text-zinc-400">{copyStatus}</span>}
          <RunStatus status={run.status} />
        </nav>
        <div className="flex shrink-0 items-center gap-2">
          <button type="button" aria-label="Instructions at execution" title="Instructions at execution" onClick={() => setInstructionsOpen(true)} className="cursor-pointer rounded-full border border-zinc-700 p-2 text-zinc-300 hover:bg-zinc-800"><svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" strokeWidth={1.5} stroke="currentColor" aria-hidden="true" className="size-4"><path strokeLinecap="round" strokeLinejoin="round" d="M19.5 14.25v-2.625a3.375 3.375 0 0 0-3.375-3.375h-1.5A1.125 1.125 0 0 1 13.5 7.125v-1.5a3.375 3.375 0 0 0-3.375-3.375H8.25m0 12.75h7.5m-7.5 3H12M10.5 2.25H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 0 0-9-9Z" /></svg></button>
          {(run.status === "queued" || run.status === "running") && <button type="button" disabled={pending} onClick={() => { void stop(); }} className="shrink-0 cursor-pointer rounded-full border border-red-500/60 px-4 py-2 text-sm text-red-400 hover:bg-red-500/10 disabled:cursor-wait disabled:opacity-50">Cancel</button>}
        </div>
      </header>
      <hr className="border-zinc-800" />
      <p className="text-sm text-zinc-400">{run.source} · Created <RelativeTime timestamp={run.createdAt} />{run.finishedAt !== null && <> · Finished <RelativeTime timestamp={run.finishedAt} /></>}</p>
      <p className="flex items-center gap-1.5 text-xs text-zinc-500"><ProviderLogo provider={run.config.provider} className="h-4 w-4 shrink-0 rounded-sm border border-zinc-800 bg-white p-0.5" /><span>{run.config.provider}/{run.config.modelId}</span><span>{run.config.thinkingLevel && ` · Thinking: ${run.config.thinkingLevel}`} · {run.config.persistent ? "Persistent context" : "Fresh context"}</span></p>
      {connectionError && <p role="status" className="text-sm text-amber-200">{connectionError}</p>}
      {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
      {detail.warnings?.map((warning, index) => <p key={`${warning.code}:${index}`} role="status" className="text-sm text-amber-200">{warning.code.replaceAll("_", " ")}: {warning.message}</p>)}
      {instructionsOpen && <InstructionsDialog instructions={detail.context?.instructions ?? workerInstructions(run.config)} onClose={() => setInstructionsOpen(false)}><p className="text-xs text-zinc-400">Insights snapshot: {detail.context?.insightStatus ?? "legacy/none"}. Stopping does not cancel exchange orders.</p>{detail.context?.insights && <div className="flex flex-wrap gap-4 text-sm">{([detail.context.insights.agent, detail.context.insights.worker]).map((snapshot) => <a className="cursor-pointer text-violet-300" key={snapshot.scope.kind} href={`/agents/${run.agentId}/memories?scope=${snapshot.scope.kind === "worker" ? encodeURIComponent(snapshot.scope.subagentId) : ""}${snapshot.version ? `&version=${snapshot.version}` : ""}`}>{snapshot.scope.kind === "agent" ? "Agent-wide" : "Worker-specific"} insights v{snapshot.version}</a>)}</div>}{detail.publications?.length ? <div className="space-y-2">{detail.publications.map((publication) => <a key={publication.ref} className="block cursor-pointer text-sm text-violet-300" href={`/agents/${encodeURIComponent(run.agentId)}/memories/${encodeURIComponent(publication.ref)}`}>{publication.ref} · {publication.kind} · {publication.summary}</a>)}</div> : null}<p className="text-xs text-zinc-500">Allowed tools: {run.config.tools.join(", ") || "None"}</p></InstructionsDialog>}
    </section>
    <div className="flex min-h-0 flex-1 flex-col gap-3">
    {run.status === "failed" && <section aria-label="Run error" className="max-h-[40%] shrink-0 space-y-2 overflow-y-auto overscroll-contain rounded-2xl border border-red-900/60 bg-red-950/10 p-4">
      <h2 className="font-medium text-red-300">Run error</h2>
      <pre className={pre}>{run.result.trim() || "No failure details recorded."}</pre>
    </section>}
    <section ref={transcriptScroll} aria-label="Conversation transcript" className={`${panel} min-h-0 flex-1 overflow-y-auto overscroll-contain${transcriptReady ? "" : " invisible"}`}>
    <h2 className="font-medium text-white">Conversation transcript</h2>
    {detail.prior.length > 0 && <div><details><summary className="cursor-pointer font-medium text-white">Earlier conversation context · {detail.prior.length} entries</summary><p className="my-4 text-sm text-zinc-400">History preceding this assignment in the persistent conversation. Later assignments are not shown.</p><Transcript entries={detail.prior} agentId={run.agentId} /></details></div>}
    {detail.entries.length ? <Transcript entries={detail.entries} agentId={run.agentId} tools={detail.tools} /> : <p className="text-sm text-zinc-500">No transcript entries yet.</p>}
    {(run.status === "queued" || run.status === "running") && <p role="status" className="flex items-center gap-2 pt-2 text-xs text-zinc-400"><svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" className="size-4 text-violet-400 animate-spin"><path d="M12 3a9 9 0 1 1-9 9" /></svg>{run.status === "queued" ? "Waiting to start" : "Thinking"}…</p>}<div ref={transcriptEnd} /></section>
    </div>
  </div>;
}
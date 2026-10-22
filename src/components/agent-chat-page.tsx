import { Link } from "@tanstack/react-router";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useEffectEvent, useId, useLayoutEffect, useRef, useState } from "react";
import type { AgentChat, ChatEntry } from "../agents/agent-service.js";
import type { ModelChoice } from "../providers/provider-service.js";
import { compactAgentChat, resetAgentChat, retryAgent, sendAgentMessage, stopAgentChat } from "../web/agents.js";
import { subagentMutations } from "../web/subagent-mutations.js";
import { ModelDialog, modelKey } from "./model-picker.js";
import { ProviderLogo } from "./provider-logo.js";
import { ToolCall } from "./tool-activity.js";
import { ArtifactContent } from "./artifact-content.js";
import { ChatBubble } from "./chat-bubble.js";
import { useAgentChat } from "./use-agent-chat.js";
import type { ChartCapture, ChartAttachment } from "../charts/contracts.js";

const compactNumber = new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 });

function money(value: number): string {
  return value > 0 && value < 0.01 ? `$${value.toFixed(4)}` : `$${value.toFixed(2)}`;
}

function ChartContextBadge({ chart }: { chart: ChartAttachment }) {
  return <p className="mb-2 rounded-lg border border-violet-900/70 bg-violet-950/20 px-3 py-2 text-xs text-violet-200" title={`Layout: ${chart.name} · Snapshot ${chart.snapshotId}`}>
    Chart · {chart.name} · {chart.coin}-PERP · {chart.interval}{chart.image ? " · Image attached" : ""}{chart.selectedDrawingId ? " · Selected object attached" : ""}
    <span className="ml-2 text-zinc-500">{new Date(chart.capturedAt).toISOString().replace("T", " ").slice(0, 19)} UTC</span>
  </p>;
}

function ChatMessage({ entry, agentId }: { entry: ChatEntry; agentId: string }) {
  if (entry.role === "tool") return entry.tool ? <ToolCall tool={entry.tool} /> : null;
  if (entry.role === "system") {
    return <div role="presentation" className="flex items-center gap-3 py-1"><span className="h-px flex-1 bg-zinc-800" /><span className="text-xs text-zinc-500">{entry.text}</span><span className="h-px flex-1 bg-zinc-800" /></div>;
  }
  return <ChatBubble role={entry.role}>
    <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-zinc-400">{entry.role === "user" ? "You" : "Assistant"}</h2>
    {entry.chart && <ChartContextBadge chart={entry.chart} />}
    {entry.failed ? <p className="whitespace-pre-wrap break-words text-sm text-red-300">{entry.text}</p> : <ArtifactContent agentId={agentId} content={entry.text} />}
  </ChatBubble>;
}

function ResetContextDialog({ onCancel, onConfirm, disabled }: { onCancel: () => void; onConfirm: () => void; disabled: boolean }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const element = dialog.current!;
    element.showModal();
    return () => element.close();
  }, []);
  return <dialog ref={dialog} aria-labelledby={titleId} aria-describedby={`${titleId}-description`} onCancel={(event) => { event.preventDefault(); onCancel(); }} className="m-auto w-[calc(100%_-_2rem)] max-w-md rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6 text-zinc-100 shadow-2xl backdrop:bg-black/70 backdrop:backdrop-blur-sm">
    <h3 id={titleId} className="text-lg font-semibold text-white">Reset context?</h3>
    <p id={`${titleId}-description`} className="mt-2 text-sm leading-6 text-zinc-400">The model will start fresh. Earlier messages remain available. Accounts, orders, and agent configuration are unchanged.</p>
    <div className="mt-6 flex flex-col-reverse gap-3 sm:flex-row sm:justify-end">
      <button type="button" autoFocus onClick={onCancel} className="cursor-pointer rounded-full border border-zinc-800 px-4 py-2 text-sm font-medium text-zinc-300 hover:border-zinc-700 hover:text-white">Cancel</button>
      <button type="button" disabled={disabled} onClick={onConfirm} className="cursor-pointer rounded-full bg-violet-600 px-4 py-2 text-sm font-medium text-white hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50">Reset context</button>
    </div>
  </dialog>;
}

export function AgentChatPage({ initial, choices, embedded = false, visible = true, chartContext }: {
  initial: AgentChat; choices: ModelChoice[]; embedded?: boolean; visible?: boolean;
  chartContext?: { label: string; disabled: boolean; capture: (image: boolean) => Promise<ChartCapture> };
}) {
  const { chat, messages, connectionError, historyLoading, historyDone, loadOlder } = useAgentChat(initial);
  const [message, setMessage] = useState("");
  const client = useQueryClient();
  const mutation = useMutation({
    mutationFn: (operation: () => Promise<void>) => operation(),
    onMutate: () => { setNotice(""); },
  });
  const pending = mutation.isPending;
  const error = mutation.error?.message;
  /** Transient outcome of a background action, such as a declined compaction. */
  const [notice, setNotice] = useState("");
  const [resetOpen, setResetOpen] = useState(false);
  const [jump, setJump] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);
  const request = useRef<{ message: string; id: string; chart?: ChartCapture } | null>(null);
  const [includeImage, setIncludeImage] = useState(false);
  const inFlight = useRef(false);
  const scroll = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  /** Scroll offset to restore after an older history page is prepended. */
  const restore = useRef<{ height: number; top: number } | null>(null);
  const id = initial.agent.id;
  useEffect(() => {
    try { setIncludeImage(localStorage.getItem(`hypervibes:chart-image:${id}`) === "true"); } catch { /* Optional preference. */ }
  }, [id]);

  useLayoutEffect(() => {
    const element = scroll.current;
    if (!element) return;
    if (restore.current) {
      element.scrollTop = restore.current.top + element.scrollHeight - restore.current.height;
      restore.current = null;
    } else if (stick.current) {
      element.scrollTop = element.scrollHeight;
    }
  }, [messages.length, chat.messages, chat.partial, visible]);
  useEffect(() => {
    const element = scroll.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [id]);
  // A compaction marker landing in the transcript answers an ongoing compaction.
  useEffect(() => {
    if (notice && chat.messages.some((entry) => entry.marker === "compaction")) setNotice("");
  }, [chat.messages, notice]);

  async function run(operation: () => Promise<void>) {
    // Guard synchronous double submits before Query's pending render commits.
    if (inFlight.current) return;
    inFlight.current = true;
    try { await mutation.mutateAsync(operation); }
    catch { /* Query retains the error and the composer keeps its request ID. */ }
    finally { inFlight.current = false; }
  }
  /** The composer keeps focus through sends so chatting stays hands-on-keyboard. */
  const composer = useRef<HTMLTextAreaElement>(null);
  async function action(kind: "send" | "retry" | "stop" | "reset" | "compact") {
    await run(async () => {
      if (kind === "compact") {
        const result = await compactAgentChat({ data: { id } });
        if (result.message) setNotice(result.message);
        return;
      }
      if (kind === "retry") {
        await retryAgent({ data: { id } });
        await Promise.all([client.invalidateQueries({ queryKey: ["agents"] }), client.invalidateQueries({ queryKey: ["agent", id] })]);
        return;
      }
      if (kind === "stop") { await stopAgentChat({ data: { id } }); return; }
      if (kind === "reset") { await resetAgentChat({ data: { id } }); return; }
      if (request.current?.message !== message) request.current = { message, id: crypto.randomUUID() };
      const outgoing = request.current;
      if (chartContext && !outgoing.chart) outgoing.chart = await chartContext.capture(includeImage && chat.supportsImages === true);
      await sendAgentMessage({ data: { id, message: outgoing.message, requestId: outgoing.id, ...(outgoing.chart ? { chart: outgoing.chart } : {}) } });
      // Committed state arrives through the event stream; no follow-up fetch.
      setMessage((current) => current === outgoing.message ? "" : current);
      if (request.current === outgoing) request.current = null;
    });
    if (kind === "send" || kind === "retry" || kind === "reset") composer.current?.focus();
  }
  const onEscape = useEffectEvent((event: KeyboardEvent) => {
    if (!visible || event.key !== "Escape" || event.repeat || event.isComposing || event.defaultPrevented || !chat.busy || pending || document.querySelector("dialog[open]")) return;
    event.preventDefault();
    void action("stop");
  });
  useEffect(() => {
    document.addEventListener("keydown", onEscape);
    return () => document.removeEventListener("keydown", onEscape);
  }, []);
  function send() {
    void action("send");
  }
  function onComposerKey(event: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      if (!disabled && message.trim()) send();
    }
  }
  async function onLoadOlder() {
    const element = scroll.current;
    const marker = element ? { height: element.scrollHeight, top: element.scrollTop } : null;
    if (await loadOlder() && marker) restore.current = marker;
  }
  function onScroll() {
    const element = scroll.current;
    if (!element) return;
    const atBottom = element.scrollHeight - element.scrollTop - element.clientHeight < 80;
    stick.current = atBottom;
    setJump(!atBottom);
    if (element.scrollTop < 400) void onLoadOlder();
  }
  function jumpToLatest() {
    const element = scroll.current;
    if (element) element.scrollTop = element.scrollHeight;
  }
  /** The orchestrator's model is the `main:` orchestrator section; the backend rejects edits while it is working. */
  async function changeModel(value: string, thinkingLevel: string) {
    const choice = choices.find((entry) => modelKey(entry) === value);
    if (!choice) throw new Error("Select an available model.");
    await mutation.mutateAsync(async () => {
      await subagentMutations(client, id).saveSection({ id: `main:${id}`, section: "model", settings: { provider: choice.provider, modelId: choice.modelId, thinkingLevel } });
      setModelOpen(false);
    });
  }
  // A modal dialog makes the rest of the page inert and returns focus to its
  // opener on close, so the composer is focused only after the dialog unmounted.
  useEffect(() => {
    if (!modelOpen) return;
    return () => { composer.current?.focus(); };
  }, [modelOpen]);
  const busy = chat.busy;
  const modelBusy = busy || chat.queuedMessages.length > 0;
  const selectedModel = choices.find((choice) => modelKey(choice) === modelKey(chat.model));
  const percentage = chat.context ? Math.min(100, Math.round((chat.context.tokens / chat.context.window) * 100)) : undefined;
  const disabled = pending || !chat.agent.enabled || !chat.configured || chat.agent.state !== "ready" || chartContext?.disabled === true;
  return (
    <section aria-label="Chat" className={`flex min-h-0 w-full flex-col overflow-hidden rounded-2xl border border-zinc-800 bg-[#0d0d10] ${embedded ? "h-full" : "h-[calc(100dvh-4.5rem)] sm:h-[calc(100dvh-5rem)]"}`}>
      {!embedded && <header className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-zinc-800 px-4 py-2 sm:px-6">
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-medium text-white">Main conversation</h2>
          <button type="button" disabled={modelBusy} onClick={() => setModelOpen(true)} aria-haspopup="dialog" aria-label="Choose model" title={modelBusy ? "Stop or finish queued work before changing the model" : "Change the model"} className="mt-0.5 flex min-w-0 max-w-full cursor-pointer items-center gap-2 rounded-lg px-1.5 py-0.5 text-xs text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200 disabled:cursor-not-allowed disabled:opacity-50">
            {selectedModel && <ProviderLogo provider={selectedModel.provider} className="h-5 w-5 shrink-0 rounded-sm border border-zinc-800 bg-white p-0.5" />}
            <span className="min-w-0 truncate">{selectedModel ? selectedModel.name : `${chat.model.provider}/${chat.model.modelId}`}{chat.thinkingLevel !== undefined && <span className="text-zinc-500"> · Thinking: {chat.thinkingLevel || "Default"}</span>}</span>
          </button>
        </div>
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-zinc-400">
          {chat.context && <span className="flex items-center gap-2" title="Estimated context usage of the next request">
            <span className="h-1.5 w-20 overflow-hidden rounded-full bg-zinc-800"><span className={`block h-full rounded-full ${percentage! >= 80 ? "bg-amber-400" : "bg-violet-500"}`} style={{ width: `${percentage}%` }} /></span>
            {compactNumber.format(chat.context.tokens)} / {compactNumber.format(chat.context.window)} tokens
          </span>}
          <span title="Lifetime conversation usage, including before context resets">Total usage: {compactNumber.format(chat.usage.input)} in · {compactNumber.format(chat.usage.output)} out{chat.usage.cost > 0 ? ` · ${money(chat.usage.cost)}` : ""}</span>
        </div>
        <div className="flex items-center gap-2">
          <button type="button" disabled={disabled || chat.compacting} onClick={() => { void action("compact"); }} className="cursor-pointer rounded-lg border border-zinc-700 px-3 py-1.5 text-xs text-zinc-300 hover:bg-zinc-800 hover:text-white disabled:cursor-not-allowed disabled:opacity-50">{chat.compacting ? "Compacting…" : "Compact"}</button>
          <button type="button" disabled={disabled} onClick={() => setResetOpen(true)} aria-haspopup="dialog" className="cursor-pointer rounded-lg border border-zinc-700 px-3 py-1.5 text-xs text-zinc-300 hover:bg-zinc-800 hover:text-white disabled:cursor-not-allowed disabled:opacity-50">Reset context</button>
        </div>
      </header>}
      {resetOpen && <ResetContextDialog disabled={disabled} onCancel={() => setResetOpen(false)} onConfirm={() => { setResetOpen(false); void action("reset"); }} />}
      {modelOpen && <ModelDialog choices={choices} value={modelKey(chat.model)} thinkingLevel={chat.thinkingLevel} onCancel={() => setModelOpen(false)} onApply={changeModel} />}
      {!chat.agent.enabled && <p role="status" className="border-b border-zinc-800 px-4 py-2 text-sm text-amber-200">Agent disabled. Enable it to chat or run workers. Exchange orders and positions remain unchanged.</p>}
      {!chat.configured && <p role="status" className="border-b border-zinc-800 px-4 py-2 text-sm text-amber-200 sm:px-6">Reconnect this Agent’s provider or select an available model. <a href="/providers" className="cursor-pointer underline">Manage providers</a></p>}
      {chat.agent.state !== "ready" && <div role="status" className="border-b border-zinc-800 px-4 py-3 sm:px-6"><p>{chat.agent.error ?? (chat.agent.state === "stopping" || chat.agent.state === "stopped" ? "Deletion pending. This agent will not resume execution." : "Initializing Agent…")}</p>{chat.agent.state === "stopping" || chat.agent.state === "stopped" ? <Link to="/agents/$agentId/settings" params={{ agentId: chat.agent.id }} className="mt-2 inline-block cursor-pointer text-violet-300 underline">Retry deletion in Settings</Link> : <button disabled={pending} onClick={() => { void action("retry"); }} className="mt-2 cursor-pointer rounded-lg bg-violet-600 px-4 py-2 text-sm disabled:cursor-wait">Retry initialization</button>}</div>}
      {chat.compactionError && <p role="status" className="border-b border-zinc-800 px-4 py-2 text-xs text-amber-200 sm:px-6">Compaction failed: {chat.compactionError}</p>}
      <div ref={scroll} onScroll={onScroll} aria-label="Conversation" aria-live="polite" className={`min-h-0 flex-1 space-y-3 overflow-y-auto ${embedded ? "px-3 py-3" : "px-4 py-4 sm:px-6"}`}>
        {!historyDone && chat.history && <div className="py-1 text-center"><button type="button" disabled={historyLoading} onClick={() => { void onLoadOlder(); }} className="cursor-pointer rounded-lg px-3 py-1 text-xs text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200 disabled:cursor-wait disabled:opacity-50">{historyLoading ? "Loading earlier messages…" : messages[0]?.marker === "reset" ? "View messages before reset" : "Load earlier messages"}</button></div>}
        {messages.length === 0 && <p className="py-8 text-center text-sm text-zinc-500">Start a conversation with your Agent.</p>}
        {messages.map((entry) => <ChatMessage key={entry.id} entry={entry} agentId={chat.agent.id} />)}
        {busy && <article className="mr-8 whitespace-pre-wrap break-words rounded-xl bg-zinc-900 p-4 text-sm text-zinc-300">{chat.partial || "Thinking…"}</article>}
        {chat.queuedMessages.map((item) => (
          <article key={item.id} className="ml-8 rounded-xl bg-violet-950/40 p-4">
            <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-zinc-400">You</h2>
            {item.chart && <ChartContextBadge chart={item.chart} />}
            <p title={item.text} className="line-clamp-6 whitespace-pre-wrap break-words text-sm text-zinc-100">{item.text}</p>
            <p className="mt-2 text-xs text-violet-300/70">Queued · sent when the response finishes</p>
          </article>
        ))}
      </div>
      <div role="status" className={`flex flex-wrap items-center gap-3 border-t border-zinc-800 text-xs text-zinc-400 ${embedded ? "px-3 py-1.5" : "px-4 py-2 sm:px-6"}`}>
        <span>{busy ? "Working" : "Idle"}{chat.queuedMessages.length > 0 ? ` · ${chat.queuedMessages.length} queued` : ""}</span>
        {notice && <span className="text-amber-200">{notice}</span>}
        {busy && <button disabled={pending} onClick={() => { void action("stop"); }} title="Stop response (Esc)" aria-keyshortcuts="Escape" className="cursor-pointer text-zinc-300 hover:text-white hover:underline disabled:cursor-wait">Stop response</button>}
        {jump && <button type="button" onClick={jumpToLatest} className="ml-auto cursor-pointer text-sky-300 hover:underline">Jump to latest</button>}
      </div>
      <form onSubmit={(event) => { event.preventDefault(); if (!disabled && message.trim()) send(); }} className={`border-t border-zinc-800 ${embedded ? "px-3 py-2" : "px-4 py-3 sm:px-6"}`}>
        {chartContext && <div className="mb-2 flex flex-wrap items-center justify-between gap-2 text-xs text-zinc-400">
          <span className={embedded ? "min-w-0 flex-1 truncate" : undefined} title={chartContext.label}>{embedded ? "Chart context" : `Attach chart context · ${chartContext.label}`}</span>
          <label className={chat.supportsImages ? "cursor-pointer" : "cursor-not-allowed text-zinc-600"} title={chat.supportsImages ? "Include this chart and visible drawings as a PNG" : "The selected model does not support images"}>
            <input type="checkbox" checked={(request.current?.chart ? Boolean(request.current.chart.image) : includeImage) && chat.supportsImages === true} disabled={!chat.supportsImages || pending || Boolean(request.current?.chart)} onChange={(event) => { setIncludeImage(event.target.checked); try { localStorage.setItem(`hypervibes:chart-image:${id}`, String(event.target.checked)); } catch { /* Optional preference. */ } }} className="mr-2 cursor-pointer disabled:cursor-not-allowed" />Include chart image
          </label>
        </div>}
        <div className={`flex items-end ${embedded ? "gap-2" : "gap-3"}`}>
          <label htmlFor="agent-message" className="sr-only">Message</label>
          <textarea id="agent-message" ref={composer} autoFocus value={message} onChange={(event) => { setMessage(event.target.value); if (request.current?.message !== event.target.value) request.current = null; }} onKeyDown={onComposerKey} required maxLength={16000} rows={1} disabled={!chat.agent.enabled || !chat.configured || chat.agent.state !== "ready"} placeholder={embedded ? "Ask about this chart" : "Message this agent"} className="field-sizing-content max-h-48 min-h-11 min-w-0 flex-1 resize-none rounded-2xl border border-zinc-800 bg-zinc-950 px-4 py-3 text-sm text-zinc-100 placeholder-zinc-600 focus:border-zinc-600 focus:outline-none disabled:cursor-not-allowed disabled:opacity-50" />
          <button type="submit" disabled={disabled || !message.trim()} aria-label={pending ? "Please wait…" : busy ? "Queue message" : "Send message"} title={busy ? "Queue message" : "Send message"} className={`shrink-0 cursor-pointer rounded-full bg-violet-600 text-sm font-medium hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50 ${embedded ? "flex size-10 items-center justify-center" : "px-5 py-2.5"}`}>{embedded ? <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" className="size-5"><path d="M12 19V5m-6 6 6-6 6 6" /></svg> : pending ? "Please wait…" : busy ? "Queue message" : "Send message"}</button>
        </div>
        {!embedded && <p className="mt-2 text-xs text-zinc-500">{busy ? "The agent is responding · Esc to stop · Enter queues your message · Shift+Enter for a new line" : "Enter to send · Shift+Enter for a new line"} · Up to 16,000 characters</p>}
      </form>
      {error && request.current?.chart && <p className="px-4 py-2 text-xs text-zinc-400">Retries reuse the captured chart and image setting. Edit your message to capture a new view.</p>}
      {(error || connectionError) && <p role="alert" className="border-t border-zinc-800 px-4 py-2 text-sm text-red-300 sm:px-6">{error || connectionError}</p>}
    </section>
  );
}
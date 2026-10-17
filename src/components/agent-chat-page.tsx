import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { AgentChat, ChatEntry } from "../agents/agent-service.js";
import type { ModelChoice } from "../providers/provider-service.js";
import { compactAgentChat, resetAgentChat, retryAgent, sendAgentMessage, stopAgentChat } from "../web/agents.js";
import { saveSubagentSection } from "../web/subagents.js";
import { ModelDialog, modelKey } from "./model-picker.js";
import { ProviderLogo } from "./provider-logo.js";
import { useAgentChat } from "./use-agent-chat.js";

const compactNumber = new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 });

function money(value: number): string {
  return value > 0 && value < 0.01 ? `$${value.toFixed(4)}` : `$${value.toFixed(2)}`;
}

function ChatMessage({ entry }: { entry: ChatEntry }) {
  if (entry.role === "system") {
    return <div role="presentation" className="flex items-center gap-3 py-1"><span className="h-px flex-1 bg-zinc-800" /><span className="text-xs text-zinc-500">{entry.text}</span><span className="h-px flex-1 bg-zinc-800" /></div>;
  }
  return <article className={`rounded-xl p-4 ${entry.role === "user" ? "ml-8 bg-violet-950/40" : "mr-8 bg-zinc-900"}`}>
    <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-zinc-400">{entry.role === "user" ? "You" : "Assistant"}</h2>
    <p className={`whitespace-pre-wrap break-words text-sm ${entry.failed ? "text-red-300" : "text-zinc-100"}`}>{entry.text}</p>
  </article>;
}

export function AgentChatPage({ initial, choices }: { initial: AgentChat; choices: ModelChoice[] }) {
  const { chat, messages, connectionError, historyLoading, historyDone, loadOlder } = useAgentChat(initial);
  const [message, setMessage] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  /** Transient outcome of a background action, such as a declined compaction. */
  const [notice, setNotice] = useState("");
  const [armed, setArmed] = useState(false);
  const [jump, setJump] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);
  const request = useRef<{ message: string; id: string } | null>(null);
  const inFlight = useRef(false);
  const scroll = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  /** Scroll offset to restore after an older history page is prepended. */
  const restore = useRef<{ height: number; top: number } | null>(null);
  const id = initial.agent.id;

  useLayoutEffect(() => {
    const element = scroll.current;
    if (!element) return;
    if (restore.current) {
      element.scrollTop = restore.current.top + element.scrollHeight - restore.current.height;
      restore.current = null;
    } else if (stick.current) {
      element.scrollTop = element.scrollHeight;
    }
  }, [messages.length, chat.partial]);
  useEffect(() => {
    const element = scroll.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [id]);
  // A compaction marker landing in the transcript answers an ongoing compaction.
  useEffect(() => {
    if (notice && chat.messages.some((entry) => entry.marker === "compaction")) setNotice("");
  }, [chat.messages, notice]);

  async function run<T extends { ok: boolean; error?: string }>(operation: () => Promise<T>): Promise<T | undefined> {
    if (inFlight.current) return undefined;
    inFlight.current = true;
    setPending(true);
    setError("");
    setNotice("");
    try {
      const result = await operation();
      if (!result.ok) setError((result as { error?: string }).error ?? "Agent operation failed. Please try again.");
      return result;
    } catch { setError("Unable to complete the request. Retrying a message will not submit it twice."); return undefined; }
    finally { inFlight.current = false; setPending(false); }
  }
  /** The composer keeps focus through sends so chatting stays hands-on-keyboard. */
  const composer = useRef<HTMLTextAreaElement>(null);
  async function action(kind: "send" | "retry" | "stop" | "reset" | "compact") {
    if (kind === "compact") {
      const result = await run(() => compactAgentChat({ data: { id } }));
      if (result?.ok && result.data.message) setNotice(result.data.message);
      return;
    }
    await run((): Promise<{ ok: boolean; error?: string }> => {
      if (kind === "send" && request.current?.message !== message) request.current = { message, id: crypto.randomUUID() };
      if (kind === "retry") return retryAgent({ data: { id } });
      if (kind === "stop") return stopAgentChat({ data: { id } });
      if (kind === "reset") return resetAgentChat({ data: { id } });
      const sent = sendAgentMessage({ data: { id, message, requestId: request.current!.id } });
      // Committed state arrives through the event stream; no follow-up fetch.
      sent.then((result) => { if (result.ok) { setMessage(""); request.current = null; } }, () => { /* run reports the failure. */ });
      return sent;
    });
    if (kind === "send" || kind === "retry" || kind === "reset") composer.current?.focus();
  }
  function send() {
    void action("send");
  }
  function onComposerKey(event: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      if (!pending && message.trim() && chat.configured && chat.agent.state === "ready") send();
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
    const result = await saveSubagentSection({ data: { agentId: id, id: `main:${id}`, section: "model", settings: { provider: choice.provider, modelId: choice.modelId, thinkingLevel } } });
    if (!result.ok) throw new Error(result.error);
    setModelOpen(false);
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
  const disabled = pending || !chat.configured || chat.agent.state !== "ready";
  return (
    <section aria-label="Chat" className="flex h-[calc(100dvh-4.5rem)] min-h-0 w-full flex-col overflow-hidden rounded-2xl border border-zinc-800 bg-[#0d0d10] sm:h-[calc(100dvh-5rem)]">
      <header className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-zinc-800 px-4 py-2 sm:px-6">
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
          <span title="Total conversation tokens and cost">{compactNumber.format(chat.usage.input)} in · {compactNumber.format(chat.usage.output)} out{chat.usage.cost > 0 ? ` · ${money(chat.usage.cost)}` : ""}</span>
        </div>
        <div className="flex items-center gap-2">
          <button type="button" disabled={disabled || chat.compacting} onClick={() => { void action("compact"); }} className="cursor-pointer rounded-lg border border-zinc-700 px-3 py-1.5 text-xs text-zinc-300 hover:bg-zinc-800 hover:text-white disabled:cursor-not-allowed disabled:opacity-50">{chat.compacting ? "Compacting…" : "Compact"}</button>
          <button type="button" disabled={disabled} onClick={() => { if (armed) { setArmed(false); void action("reset"); } else setArmed(true); }} onBlur={() => setArmed(false)} className="cursor-pointer rounded-lg border border-zinc-700 px-3 py-1.5 text-xs text-zinc-300 hover:bg-zinc-800 hover:text-white disabled:cursor-not-allowed disabled:opacity-50">{armed ? "Confirm new chat?" : "New chat"}</button>
        </div>
      </header>
      {modelOpen && <ModelDialog choices={choices} value={modelKey(chat.model)} thinkingLevel={chat.thinkingLevel} onCancel={() => setModelOpen(false)} onApply={changeModel} />}
      {!chat.configured && <p role="status" className="border-b border-zinc-800 px-4 py-2 text-sm text-amber-200 sm:px-6">Reconnect this Agent’s provider or select an available model. <a href="/providers" className="cursor-pointer underline">Manage providers</a></p>}
      {chat.agent.state !== "ready" && <div role="status" className="border-b border-zinc-800 px-4 py-3 sm:px-6"><p>{chat.agent.error ?? "Initializing Agent…"}</p><button disabled={pending} onClick={() => { void action("retry"); }} className="mt-2 cursor-pointer rounded-lg bg-violet-600 px-4 py-2 text-sm disabled:cursor-wait">Retry initialization</button></div>}
      {chat.compactionError && <p role="status" className="border-b border-zinc-800 px-4 py-2 text-xs text-amber-200 sm:px-6">Compaction failed: {chat.compactionError}</p>}
      <div ref={scroll} onScroll={onScroll} aria-label="Conversation" aria-live="polite" className="min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-4 sm:px-6">
        {!historyDone && chat.history && <p className="py-1 text-center text-xs text-zinc-600">{historyLoading ? "Loading earlier messages…" : "Scroll up to load earlier messages"}</p>}
        {messages.length === 0 && <p className="py-8 text-center text-sm text-zinc-500">Start a conversation with your Agent.</p>}
        {messages.map((entry) => <ChatMessage key={entry.id} entry={entry} />)}
        {busy && <article className="mr-8 whitespace-pre-wrap break-words rounded-xl bg-zinc-900 p-4 text-sm text-zinc-300">{chat.partial || "Thinking…"}</article>}
        {chat.queuedMessages.map((item) => (
          <article key={item.id} className="ml-8 rounded-xl bg-violet-950/40 p-4">
            <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-zinc-400">You</h2>
            <p title={item.text} className="line-clamp-6 whitespace-pre-wrap break-words text-sm text-zinc-100">{item.text}</p>
            <p className="mt-2 text-xs text-violet-300/70">Queued · sent when the response finishes</p>
          </article>
        ))}
      </div>
      <div role="status" className="flex items-center gap-3 border-t border-zinc-800 px-4 py-2 text-xs text-zinc-400 sm:px-6">
        <span>{busy ? "Working" : "Idle"}{chat.queuedMessages.length > 0 ? ` · ${chat.queuedMessages.length} queued` : ""}</span>
        {notice && <span className="text-amber-200">{notice}</span>}
        {busy && <button disabled={pending} onClick={() => { void action("stop"); }} className="cursor-pointer text-red-300 hover:underline disabled:cursor-wait">Stop response</button>}
        {jump && <button type="button" onClick={jumpToLatest} className="ml-auto cursor-pointer text-sky-300 hover:underline">Jump to latest</button>}
      </div>
      <form onSubmit={(event) => { event.preventDefault(); if (!disabled && message.trim()) send(); }} className="border-t border-zinc-800 px-4 py-3 sm:px-6">
        <div className="flex items-end gap-3">
          <label htmlFor="agent-message" className="sr-only">Message</label>
          <textarea id="agent-message" ref={composer} autoFocus value={message} onChange={(event) => setMessage(event.target.value)} onKeyDown={onComposerKey} required maxLength={16000} rows={1} disabled={!chat.configured || chat.agent.state !== "ready"} placeholder="Message this agent" className="field-sizing-content max-h-48 min-h-11 flex-1 resize-none rounded-2xl border border-zinc-800 bg-zinc-950 px-4 py-3 text-sm text-zinc-100 placeholder-zinc-600 focus:border-zinc-600 focus:outline-none disabled:cursor-not-allowed disabled:opacity-50" />
          <button type="submit" disabled={disabled || !message.trim()} className="shrink-0 cursor-pointer rounded-full bg-violet-600 px-5 py-2.5 text-sm font-medium hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50">{pending ? "Please wait…" : busy ? "Queue message" : "Send message"}</button>
        </div>
        <p className="mt-2 text-xs text-zinc-500">{busy ? "The agent is responding · Enter queues your message · Shift+Enter for a new line" : "Enter to send · Shift+Enter for a new line"} · Up to 16,000 characters</p>
      </form>
      {(error || connectionError) && <p role="alert" className="border-t border-zinc-800 px-4 py-2 text-sm text-red-300 sm:px-6">{error || connectionError}</p>}
    </section>
  );
}
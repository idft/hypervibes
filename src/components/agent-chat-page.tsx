import { useEffect, useRef, useState } from "react";
import type { AgentChat } from "../agents/agent-service.js";
import { retryAgent, selectAgentModel, sendAgentMessage, stopAgentChat } from "../web/agents.js";
import type { ModelChoice } from "../providers/provider-service.js";
import { ModelPicker, modelKey } from "./model-picker.js";

export function AgentChatPage({ initial, choices = [] }: { initial: AgentChat; choices?: ModelChoice[] }) {
  const [chat, setChat] = useState(initial);
  const [message, setMessage] = useState("");
  const [pending, setPending] = useState(false);
  const [selection, setSelection] = useState(modelKey(initial.model));
  const [thinking, setThinking] = useState(initial.thinkingLevel ?? "");
  const selected = choices.find((choice) => modelKey(choice) === selection);
  const [error, setError] = useState("");
  const [connectionError, setConnectionError] = useState("");
  const request = useRef<{ message: string; id: string } | null>(null);
  const inFlight = useRef(false);
  const end = useRef<HTMLDivElement>(null);
  const id = initial.agent.id;
  const disconnected = useRef(false);
  useEffect(() => {
    setSelection(modelKey(chat.model));
    setThinking(chat.thinkingLevel ?? "");
  }, [chat.model.provider, chat.model.modelId, chat.thinkingLevel]);
  useEffect(() => {
    // The server pushes committed snapshots; EventSource reconnects by itself.
    const source = new EventSource(`/api/agents/${id}/events`);
    const interrupted = (event: MessageEvent | undefined, fallback: string) => {
      setConnectionError(event ? (JSON.parse(event.data) as { message?: string }).message ?? fallback : fallback);
    };
    source.addEventListener("chat", (event) => {
      setChat(JSON.parse((event as MessageEvent).data) as AgentChat);
      setConnectionError("");
    });
    source.addEventListener("unauthorized", () => { window.location.assign("/login"); });
    source.addEventListener("shutdown", () => interrupted(undefined, "Server is restarting. Reconnecting…"));
    source.addEventListener("unavailable", (event) => interrupted(event as MessageEvent, "Chat stream interrupted. Reconnecting…"));
    source.addEventListener("stream-error", (event) => interrupted(event as MessageEvent, "Conversation is too large to stream. Reload the page."));
    source.onerror = () => { if (!disconnected.current) interrupted(undefined, "Connection interrupted. Reconnecting…"); };
    return () => { disconnected.current = true; source.close(); };
  }, [id]);
  useEffect(() => { end.current?.scrollIntoView({ block: "nearest" }); }, [chat.messages.length, chat.partial]);

  async function changeModel() {
    if (inFlight.current || !selected) return;
    inFlight.current = true; setPending(true); setError("");
    try {
      const result = await selectAgentModel({ data: { id, provider: selected.provider, modelId: selected.modelId, thinkingLevel: thinking } });
      if (!result.ok) setError(result.error);
    } catch { setError("Unable to change models. Try again."); }
    finally { inFlight.current = false; setPending(false); }
  }
  async function action(kind: "send" | "retry" | "stop") {
    if (inFlight.current) return;
    inFlight.current = true;
    setPending(true);
    setError("");
    try {
      if (kind === "send" && request.current?.message !== message) request.current = { message, id: crypto.randomUUID() };
      const result = kind === "retry" ? await retryAgent({ data: { id } }) : kind === "stop" ? await stopAgentChat({ data: { id } }) : await sendAgentMessage({ data: { id, message, requestId: request.current!.id } });
      if (!result.ok) { setError(result.error); return; }
      if (kind === "send") { setMessage(""); request.current = null; }
      // Committed state arrives through the event stream; no follow-up fetch.
    } catch { setError("Unable to complete the request. Retrying a message will not submit it twice."); }
    finally { inFlight.current = false; setPending(false); }
  }
  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-5">
      <a href="/agents" className="w-fit cursor-pointer text-sm text-violet-300 hover:underline">← Agents</a>
      <header><h1 className="text-3xl font-semibold text-white">{chat.agent.name}</h1><p className="mt-2 text-sm text-zinc-400">Main conversation · {chat.model.provider}/{chat.model.modelId}</p></header>
      <p className="rounded-xl border border-amber-800/70 bg-amber-950/30 px-4 py-3 text-sm text-amber-200">Trading is not configured. This Agent has no exchange, filesystem, or shell tools.</p>
      {!chat.configured && <p role="status" className="text-sm text-amber-200">Reconnect this Agent’s provider or select an available model. <a href="/providers" className="cursor-pointer underline">Manage providers</a></p>}
      {choices.length > 0 && <div className="space-y-3 rounded-xl border border-zinc-800 p-4">
        <ModelPicker choices={choices} value={selected ? selection : ""} onChange={(value) => { setSelection(value); setThinking(""); }} disabled={pending || chat.busy || chat.queued > 0} />
        {Boolean(selected?.thinkingLevels.length) && <label className="block text-sm text-zinc-300">Thinking level
          <select aria-label="Thinking level" value={thinking} onChange={(event) => setThinking(event.target.value)} disabled={pending || chat.busy || chat.queued > 0} className="ml-3 cursor-pointer rounded-lg border border-zinc-700 bg-zinc-950 p-2">
            <option value="">Default</option>{selected?.thinkingLevels.map((level) => <option key={level} value={level}>{level}</option>)}
          </select>
        </label>}
        <button type="button" onClick={() => { void changeModel(); }} disabled={pending || chat.busy || chat.queued > 0 || !selected || chat.agent.state !== "ready"} className="cursor-pointer rounded-lg border border-zinc-700 px-3 py-2 text-sm disabled:cursor-not-allowed disabled:opacity-50">Apply model</button>
        <p className="text-xs text-zinc-500">Change models while idle. Conversation history is preserved.</p>
      </div>}
      {chat.agent.state !== "ready" && <div role="status" className="rounded-xl border border-zinc-700 p-4"><p>{chat.agent.error ?? "Initializing Agent…"}</p><button disabled={pending} onClick={() => { void action("retry"); }} className="mt-3 cursor-pointer rounded-lg bg-violet-600 px-4 py-2 text-sm disabled:cursor-wait">Retry initialization</button></div>}
      <section aria-label="Conversation" aria-live="polite" className="max-h-[55vh] min-h-64 space-y-4 overflow-y-auto rounded-xl border border-zinc-800 bg-[#0d0d10] p-5">
        {chat.messages.length === 0 && <p className="py-8 text-center text-sm text-zinc-500">Start a conversation with your Agent.</p>}
        {chat.messages.map((entry) => <article key={entry.id} className={`rounded-xl p-4 ${entry.role === "user" ? "ml-8 bg-violet-950/40" : "mr-8 bg-zinc-900"}`}><h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-zinc-400">{entry.role === "user" ? "You" : "Assistant"}</h2><p className={`whitespace-pre-wrap break-words text-sm ${entry.failed ? "text-red-300" : "text-zinc-100"}`}>{entry.text}</p></article>)}
        {chat.busy && <article className="mr-8 whitespace-pre-wrap break-words rounded-xl bg-zinc-900 p-4 text-sm text-zinc-300">{chat.partial || "Thinking…"}</article>}
        <div ref={end} />
      </section>
      <div role="status" className="flex items-center gap-3 text-xs text-zinc-400"><span>{chat.busy ? "Working" : "Idle"}{chat.queued > 0 ? ` · ${chat.queued} queued` : ""} · Showing the latest 100 transcript entries</span>{chat.busy && <button disabled={pending} onClick={() => { void action("stop"); }} className="cursor-pointer text-red-300 hover:underline disabled:cursor-wait">Stop response</button>}</div>
      <form onSubmit={(event) => { event.preventDefault(); void action("send"); }} className="flex flex-col gap-3">
        <label className="text-sm text-zinc-300" htmlFor="agent-message">Message</label>
        <textarea id="agent-message" value={message} onChange={(event) => setMessage(event.target.value)} required maxLength={16000} rows={3} disabled={pending} className="resize-y rounded-xl border border-zinc-700 bg-zinc-950 px-4 py-3 text-sm text-white" />
        <button disabled={pending || !message.trim() || !chat.configured || chat.agent.state !== "ready"} className="w-fit cursor-pointer rounded-full bg-violet-600 px-5 py-2 text-sm font-medium hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50">{pending ? "Please wait…" : chat.busy ? "Queue message" : "Send message"}</button>
      </form>
      {(error || connectionError) && <p role="alert" className="text-sm text-red-300">{error || connectionError}</p>}
    </div>
  );
}

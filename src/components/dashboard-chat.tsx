import { useRef, useState } from "react";
import { useRouter } from "@tanstack/react-router";
import { sendAgentMessage } from "../web/agents.js";

export function DashboardChat({ agentId }: { agentId: string }) {
  const router = useRouter();
  const [message, setMessage] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  const request = useRef<{ message: string; id: string } | null>(null);

  async function send() {
    if (inFlight.current || !message.trim()) return;
    inFlight.current = true;
    setPending(true);
    setError("");
    try {
      if (request.current?.message !== message) request.current = { message, id: crypto.randomUUID() };
      const result = await sendAgentMessage({ data: { id: agentId, message, requestId: request.current.id } });
      if (!result.ok) { setError(result.error); return; }
      await router.navigate({ to: "/agents/$agentId/chat", params: { agentId } });
    } catch {
      setError("Unable to complete the request. Retry with the same message to avoid submitting it twice.");
    } finally { inFlight.current = false; setPending(false); }
  }

  return <section aria-label="Chat" className="rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
    <h3 className="mb-4 text-lg font-medium text-white">Chat</h3>
    <form onSubmit={(event) => { event.preventDefault(); void send(); }}>
      <div className="flex items-end gap-3">
        <textarea aria-label="Message to orchestrator" placeholder="Message the orchestrator…" value={message} onChange={(event) => setMessage(event.target.value)} rows={2} maxLength={16000} disabled={pending} onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); void send(); }
        }} className="min-w-0 flex-1 resize-y rounded-xl border border-zinc-700 bg-zinc-950 px-4 py-3 text-sm text-zinc-100 outline-none focus:border-violet-500 disabled:opacity-50" />
        <button type="submit" disabled={pending || !message.trim()} className="shrink-0 cursor-pointer rounded-full bg-violet-600 px-5 py-2.5 text-sm font-medium text-white hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50">{pending ? "Sending…" : "Send"}</button>
      </div>
      {error && <p role="alert" className="mt-3 text-sm text-red-300">{error}</p>}
    </form>
  </section>;
}

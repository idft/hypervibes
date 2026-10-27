import { randomUuid } from "../web/random-uuid.js";
import { useRef, useState } from "react";
import { useRouter } from "@tanstack/react-router";
import { useMutation } from "@tanstack/react-query";
import { sendAgentMessage } from "../web/agents.js";

export function DashboardChat({ agentId, enabled = true }: { agentId: string; enabled?: boolean }) {
  const router = useRouter();
  const [message, setMessage] = useState("");
  const mutation = useMutation({
    mutationFn: async () => {
      if (request.current?.message !== message) request.current = { message, id: randomUuid() };
      return sendAgentMessage({ data: { id: agentId, message, requestId: request.current.id } });
    },
    onSuccess: async () => { await router.navigate({ to: "/agents/$agentId/chat", params: { agentId } }); },
  });
  const pending = mutation.isPending;
  const error = mutation.error ? `${mutation.error.message} Retry with the same message to avoid submitting it twice.` : "";
  const inFlight = useRef(false);
  const request = useRef<{ message: string; id: string } | null>(null);

  async function send() {
    if (!enabled || inFlight.current || !message.trim()) return;
    inFlight.current = true;
    try {
      await mutation.mutateAsync();
    } catch { /* Query retains the error; preserve the idempotent submission. */ }
    finally { inFlight.current = false; }
  }

  return <section aria-label="Chat" className="shrink-0 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
    <h3 className="mb-4 text-lg font-medium text-white">Chat</h3>
    {!enabled && <p role="status" className="mb-3 text-sm text-amber-200">Agent disabled. Enable it to chat or run workers. Exchange orders and positions remain unchanged.</p>}
    <form onSubmit={(event) => { event.preventDefault(); void send(); }}>
      <div className="flex items-end gap-3">
        <textarea aria-label="Message to orchestrator" placeholder="Message the orchestrator…" value={message} onChange={(event) => setMessage(event.target.value)} rows={2} maxLength={16000} disabled={pending || !enabled} onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); void send(); }
        }} className="min-w-0 flex-1 resize-none rounded-xl border border-zinc-700 bg-zinc-950 px-4 py-3 text-sm text-zinc-100 outline-none focus:border-violet-500 disabled:opacity-50" />
        <button type="submit" disabled={pending || !enabled || !message.trim()} className="shrink-0 cursor-pointer rounded-full bg-violet-600 px-5 py-2.5 text-sm font-medium text-white hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50">{pending ? "Sending…" : "Send"}</button>
      </div>
      {error && <p role="alert" className="mt-3 text-sm text-red-300">{error}</p>}
    </form>
  </section>;
}

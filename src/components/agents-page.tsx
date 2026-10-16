import { useRef, useState } from "react";
import type { AgentRow } from "../db/agent-schema.js";

export interface CreateAgentResult { ok: boolean; error?: string; data?: AgentRow }

export function AgentsPage({ agents, create }: {
  agents: AgentRow[];
  create: (name: string, requestId: string) => Promise<CreateAgentResult>;
}) {
  const [name, setName] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const request = useRef<{ name: string; id: string } | null>(null);
  const inFlight = useRef(false);
  async function submit() {
    if (inFlight.current) return;
    inFlight.current = true;
    setPending(true);
    setError("");
    try {
      if (request.current?.name !== name) request.current = { name, id: crypto.randomUUID() };
      const result = await create(name, request.current.id);
      if (!result.ok || !result.data) { setError(result.error ?? "Unable to create Agent"); return; }
      window.location.assign(`/agents/${result.data.id}`);
    } catch { setError("Unable to create Agent. Retry to safely check the same request."); }
    finally { inFlight.current = false; setPending(false); }
  }
  return (
    <div className="mx-auto max-w-[1400px]">
      <div role="status" className="mb-6 rounded-xl border border-amber-800/70 bg-amber-950/30 px-4 py-3 text-sm text-amber-200">Trading is not configured. Agents can chat, but cannot access exchange accounts or place orders.</div>
      <header className="border-b border-zinc-800 pb-6"><h1 className="text-3xl font-semibold tracking-tight text-white">Agents</h1></header>
      <form onSubmit={(event) => { event.preventDefault(); void submit(); }} className="my-6 flex flex-wrap items-end gap-3 rounded-xl border border-zinc-800 bg-[#0d0d10] p-5">
        <label className="flex min-w-48 flex-1 flex-col gap-2 text-sm text-zinc-300">Agent name
          <input value={name} onChange={(event) => setName(event.target.value)} required maxLength={80} disabled={pending} placeholder="BTC Momentum" className="rounded-lg border border-zinc-700 bg-zinc-950 px-3 py-2 text-white" />
        </label>
        <button disabled={pending || !name.trim()} className="cursor-pointer rounded-full bg-violet-600 px-5 py-2 text-sm font-medium text-white hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50">{pending ? "Creating…" : "Create a new agent"}</button>
        {error && <p role="alert" className="w-full text-sm text-red-300">{error}</p>}
      </form>
      {agents.length === 0 ? <section className="flex min-h-48 items-center justify-center rounded-xl border border-dashed border-zinc-800"><h2 className="text-xl text-white">No agents yet</h2></section> :
        <ul className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">{agents.map((agent) => <li key={agent.id}>
          <a href={`/agents/${agent.id}`} className="block cursor-pointer rounded-xl border border-zinc-800 bg-[#0d0d10] p-5 hover:border-violet-700">
            <h2 className="text-xl font-semibold text-white">{agent.name}</h2>
            <p className="mt-2 text-sm text-zinc-400">{agent.state === "ready" ? "Chat available" : agent.state === "failed" ? "Initialization failed" : "Initializing…"}</p>
            <p className="mt-1 text-xs text-amber-300">Trading not configured</p>
          </a>
        </li>)}</ul>}
    </div>
  );
}

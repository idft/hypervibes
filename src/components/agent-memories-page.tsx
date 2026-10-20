import { useEffect, useRef, useState } from "react";
import type { SubagentRow } from "../db/subagent-schema.js";
import { Link } from "@tanstack/react-router";
import type { MemoryMetadata } from "../memory/memory-contracts.js";
import type { InsightScope } from "../memory/insight-contracts.js";
import { getInsights, listMemories } from "../web/memories.js";
import { InstrumentLabel } from "./instrument-label.js";
import { MemoryStatus } from "./memory-status.js";
import { MemoryReplacements } from "./memory-replacements.js";
import { MemoryBadge } from "./memory-badge.js";
import { ToggleTimestamp } from "./toggle-timestamp.js";

const button = "cursor-pointer rounded-lg border border-zinc-700 px-3 py-2 text-sm text-zinc-200 hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-50";
const input = "rounded-lg border border-zinc-700 bg-zinc-950 p-2 text-sm text-white";
type Cursor = { createdAt: number; ref: string };
type MemoryPage = Extract<Awaited<ReturnType<typeof listMemories>>, { ok: true }>["data"];
type InsightData = Extract<Awaited<ReturnType<typeof getInsights>>, { ok: true }>["data"];
export function AgentMemoriesPage({ agentId, workers, initialMemories, scopeId = "", version, initialTab }: { initialTab?: "insights" | "memories"; agentId: string; workers: SubagentRow[]; initialMemories?: MemoryPage; scopeId?: string; version?: number }) {
  const tab = initialTab ?? (scopeId || version ? "insights" : "memories");
  const [items, setItems] = useState<MemoryMetadata[]>(initialMemories?.items ?? []), [next, setNext] = useState<Cursor | null>(initialMemories?.nextCursor ?? null);
  const [kind, setKind] = useState(""), [coin, setCoin] = useState(""), [topic, setTopic] = useState(""), [producer, setProducer] = useState(""), [status, setStatus] = useState(""), [query, setQuery] = useState(""), [start, setStart] = useState(""), [end, setEnd] = useState("");
  const [scope, setScope] = useState(scopeId), [insight, setInsight] = useState<InsightData | null>(null);
  const [busy, setBusy] = useState(false), [error, setError] = useState("");
  const inFlight = useRef(false);
  const insightScope = (id = scope): InsightScope => id ? { kind: "worker", subagentId: id } : { kind: "agent" };
  async function perform(action: () => Promise<void>) {
    if (inFlight.current) return;
    inFlight.current = true; setBusy(true); setError("");
    try { await action(); } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to load data"); }
    finally { inFlight.current = false; setBusy(false); }
  }
  async function load(cursor?: Cursor) {
    const filters = { ...(kind ? { kind } : {}), ...(coin ? { instrument: coin } : {}), ...(topic ? { topic } : {}), ...(producer ? { producerSubagentId: producer } : {}), ...(status ? { status } : {}), ...(query ? { query } : {}), ...(start ? { start: Date.parse(`${start}T00:00:00Z`) } : {}), ...(end ? { end: Date.parse(`${end}T00:00:00Z`) } : {}), ...(cursor ? { cursor } : {}) };
    const result = await listMemories({ data: { agentId, filters } });
    if (!result.ok) throw new Error(result.error);
    setItems(result.data.items); setNext(result.data.nextCursor);
  }
  async function loadInsight(id = scope, beforeVersion?: number) {
    const result = await getInsights({ data: { agentId, scope: insightScope(id), history: true, ...(beforeVersion === undefined ? {} : { beforeVersion }) } });
    if (!result.ok) throw new Error(result.error);
    setInsight(result.data);
  }
  useEffect(() => { void perform(async () => { if (tab === "insights") { setScope(scopeId); setInsight(null); await loadInsight(scopeId); } else if (!initialMemories) await load(); }); }, [agentId, scopeId, tab, initialMemories]);
  function changeScope(id: string) {
    if (inFlight.current || id === scope) return;
    setScope(id); setInsight(null); void perform(() => loadInsight(id));
  }
  return <section className="w-full min-w-0 space-y-4 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6 text-zinc-200">
    <header><h1 className="text-lg font-medium text-white">{tab === "memories" ? "Memories" : "Insights"}</h1></header>
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
    {tab === "memories" ? <>
      <details>
        <summary className="w-fit cursor-pointer text-sm text-zinc-400 hover:text-white">Filter</summary>
      <form className="mt-3 flex flex-wrap items-end gap-3 text-sm" onSubmit={(event) => { event.preventDefault(); void perform(() => load()); }}>
        <label>Kind<select aria-label="Memory kind" className={`${input} ml-2 cursor-pointer`} value={kind} onChange={(event) => setKind(event.target.value)}><option value="">All</option>{["research", "decision", "review"].map((value) => <option key={value}>{value}</option>)}</select></label>
        <label>Instrument<input className={`${input} ml-2 w-24`} maxLength={32} value={coin} onChange={(event) => setCoin(event.target.value)} /></label><label>Topic<input className={`${input} ml-2 w-40`} maxLength={64} value={topic} onChange={(event) => setTopic(event.target.value)} /></label>
        <label>Producer<select className={`${input} ml-2 cursor-pointer`} value={producer} onChange={(event) => setProducer(event.target.value)}><option value="">All</option>{workers.filter((worker) => !worker.archived).map((worker) => <option key={worker.id} value={worker.id}>{worker.name}</option>)}</select></label>
        <label>Status<select className={`${input} ml-2 cursor-pointer`} value={status} onChange={(event) => setStatus(event.target.value)}><option value="">All</option>{["fresh", "stale", "retracted", "invalid_evidence", "durable"].map((value) => <option key={value}>{value}</option>)}</select></label>
        <label>From (UTC)<input type="date" className={`${input} ml-2`} value={start} onChange={(event) => setStart(event.target.value)} /></label><label>Until exclusive (UTC)<input type="date" className={`${input} ml-2`} value={end} onChange={(event) => setEnd(event.target.value)} /></label>
        <input aria-label="Search memory summaries" placeholder="Search summaries" className={input} maxLength={200} value={query} onChange={(event) => setQuery(event.target.value)} /><button className={button} disabled={busy}>Apply filters</button>
      </form>
      </details>
      <div className="space-y-4"><section className="min-w-0 space-y-4" aria-label="Memory publications">
        <div className="overflow-x-auto rounded-xl border border-zinc-800">
          <table className="w-full text-left text-sm" aria-label="Memories" aria-busy={busy}>
            <thead className="border-b border-zinc-800 bg-zinc-900/50 text-xs text-zinc-400"><tr>{["Memory", "Kind", "Instrument", "Topic / interval", "Status", "Producer", "Created"].map((label) => <th key={label} scope="col" className="whitespace-nowrap px-4 py-3 font-medium">{label}</th>)}</tr></thead>
            <tbody className="divide-y divide-zinc-800">{items.map((item) => <tr key={item.ref} className="hover:bg-zinc-900/50">
              <td className="min-w-64 px-4 py-3"><Link to="/agents/$agentId/memories/$memoryRef" params={{ agentId, memoryRef: item.ref }} className="block w-full cursor-pointer space-y-1 text-left hover:text-violet-300"><span className="block font-medium text-violet-300">{item.ref}</span><span className="block break-words">{item.summary}</span></Link></td>
              <td className="px-4 py-3 align-top"><MemoryBadge value={item.kind} /></td><td className="whitespace-nowrap px-4 py-3 align-top">{item.instrument ? <InstrumentLabel symbol={item.instrument} iconOnly /> : "Agent-wide"}</td>
              <td className="px-4 py-3 align-top"><MemoryBadge value={item.topic} /><span className="mt-1 block text-xs text-zinc-500">{item.interval ?? "—"}</span></td>
              <td className="px-4 py-3 align-top"><MemoryStatus status={item.status} /><MemoryReplacements agentId={agentId} memory={item} /></td>
              <td className="px-4 py-3 align-top">{workers.find((worker) => worker.id === item.producerSubagentId)?.name ?? "Root"}</td>
              <td className="whitespace-nowrap px-4 py-3 align-top text-xs text-zinc-400"><ToggleTimestamp timestamp={item.createdAt} /></td>
            </tr>)}{!items.length && <tr><td colSpan={7} className="px-4 py-8 text-center text-zinc-400">No memories found. Try different filters, or configure an analysis/review worker and its memory tools.</td></tr>}</tbody>
          </table>
        </div>
        <button className={button} disabled={busy || !next} onClick={() => { if (next) void perform(() => load(next)); }}>Next page</button>
      </section>
      </div>
    </> : <section className="space-y-4">
      <p className="text-sm text-zinc-400">Insights are read-only, managed by a Review sub-agent</p>
      <div className="grid min-h-80 grid-cols-1 overflow-hidden rounded-xl border border-zinc-800 md:grid-cols-[14rem_minmax(0,1fr)]">
        <nav aria-label="Insights scope" className="space-y-1 border-b border-zinc-800 bg-zinc-900/30 p-3 md:border-r md:border-b-0">
          {[{ id: "", name: "Global" }, ...workers.filter((worker) => worker.kind === "worker" && !worker.archived)].map((item) => <button key={item.id} type="button" aria-current={scope === item.id ? "true" : undefined} disabled={busy} onClick={() => changeScope(item.id)} className={`block w-full cursor-pointer rounded-lg px-3 py-2 text-left text-sm disabled:cursor-not-allowed ${scope === item.id ? "bg-violet-500/10 text-violet-300" : "text-zinc-400 hover:bg-zinc-800 hover:text-white"}`}>{item.name}</button>)}
        </nav>
        <article aria-label="Latest insights" aria-busy={busy} className="min-w-0 space-y-5 p-5">
          <header className="flex flex-wrap items-center justify-between gap-2"><h2 className="font-medium text-white">{scope ? workers.find((worker) => worker.id === scope)?.name ?? "Sub-agent" : "Global"}</h2>{insight && <span className="text-xs text-zinc-500">Latest · v{insight.current.version}</span>}</header>
          {insight ? <>
            <pre className="min-h-40 whitespace-pre-wrap break-words font-sans text-sm leading-relaxed">{insight.current.content || "No insights yet."}</pre>
            {insight.head && <p className="text-xs text-zinc-500">Updated {new Date(insight.head.createdAt).toISOString()}</p>}
            <details key={scope} className="border-t border-zinc-800 pt-4">
              <summary className="cursor-pointer text-sm text-zinc-400 hover:text-white">Revision history</summary>
              <ul className="mt-3 space-y-3 text-xs text-zinc-400">{insight.history?.items.map((item) => <li key={item.version}><span className="text-zinc-200">v{item.version}</span> · {new Date(item.createdAt).toISOString()}<p className="mt-1">{item.reason}</p></li>)}</ul>
              {!insight.history?.items.length && <p className="mt-3 text-sm text-zinc-500">No revisions yet.</p>}
              {insight.history?.nextBeforeVersion && <button className={`${button} mt-3`} disabled={busy} onClick={() => { void perform(() => loadInsight(scope, insight.history!.nextBeforeVersion!)); }}>Older revisions</button>}
            </details>
          </> : <p className="text-sm text-zinc-500">{busy ? "Loading insights…" : "Insights unavailable."}</p>}
        </article>
      </div>
    </section>}
  </section>;
}

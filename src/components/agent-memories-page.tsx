import { useEffect, useRef, useState } from "react";
import type { SubagentRow } from "../db/subagent-schema.js";
import type { MemoryDetail, MemoryMetadata } from "../memory/memory-contracts.js";
import type { InsightScope, InsightSnapshot } from "../memory/insight-contracts.js";
import { getInsights, getMemory, listMemories, replaceInsights } from "../web/memories.js";
import { ArtifactPreviewDialog } from "./artifact-preview-dialog.js";

const button = "cursor-pointer rounded-lg border border-zinc-700 px-3 py-2 text-sm text-zinc-200 hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-50";
const input = "rounded-lg border border-zinc-700 bg-zinc-950 p-2 text-sm text-white";
const panel = "space-y-4 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-5";
type Cursor = { createdAt: number; ref: string };
type InsightData = Extract<Awaited<ReturnType<typeof getInsights>>, { ok: true }>["data"];
export function AgentMemoriesPage({ agentId, workers, memory, scopeId = "", version, initialTab }: { initialTab?: "insights" | "memories"; agentId: string; workers: SubagentRow[]; memory?: string; scopeId?: string; version?: number }) {
  const tab = initialTab ?? (scopeId || version ? "insights" : "memories");
  const [items, setItems] = useState<MemoryMetadata[]>([]), [next, setNext] = useState<Cursor | null>(null), [selected, setSelected] = useState<MemoryDetail | null>(null);
  const [kind, setKind] = useState(""), [coin, setCoin] = useState(""), [topic, setTopic] = useState(""), [producer, setProducer] = useState(""), [status, setStatus] = useState(""), [query, setQuery] = useState(""), [start, setStart] = useState(""), [end, setEnd] = useState("");
  const [scope, setScope] = useState(scopeId), [insight, setInsight] = useState<InsightData | null>(null), [draft, setDraft] = useState(""), [base, setBase] = useState(0), [reason, setReason] = useState(""), [refs, setRefs] = useState("");
  const [busy, setBusy] = useState(false), [error, setError] = useState("");
  const [artifactId, setArtifactId] = useState<string | null>(null);
  const mutation = useRef<{ body: string; id: string } | null>(null);
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
  async function open(ref: string) {
    const result = await getMemory({ data: { agentId, ref } });
    if (!result.ok) throw new Error(result.error);
    setSelected(result.data);
    if (typeof window !== "undefined") window.history.replaceState(null, "", `?memory=${encodeURIComponent(ref)}`);
  }
  async function loadInsight(id = scope, revision?: number, beforeVersion?: number, keepDraft = false) {
    const result = await getInsights({ data: { agentId, scope: insightScope(id), history: true, ...(revision === undefined ? {} : { version: revision }), ...(beforeVersion === undefined ? {} : { beforeVersion }) } });
    if (!result.ok) throw new Error(result.error);
    setInsight(result.data);
    if (!keepDraft) { setDraft(result.data.current.content); setBase(result.data.current.version); setReason(""); setRefs(result.data.head?.evidenceRefs.join(", ") ?? ""); mutation.current = null; }
  }
  useEffect(() => { void perform(async () => { await load(); if (memory) await open(memory); if (initialTab === "insights" || scopeId || version) await loadInsight(scopeId, version); }); }, [agentId, memory, scopeId, version, initialTab]);
  async function save() {
    const data = { scope: insightScope(), expectedVersion: base, content: draft, reason, evidenceRefs: refs.split(/\s*,\s*|\s+/).filter(Boolean) };
    const body = JSON.stringify(data);
    if (mutation.current?.body !== body) mutation.current = { body, id: crypto.randomUUID() };
    const result = await replaceInsights({ data: { agentId, requestId: mutation.current!.id, input: data } });
    if (!result.ok) throw new Error(`${result.error}. Your draft and request identity have been preserved. Refresh/reconcile before replacing a newer head.`);
    await loadInsight();
  }
  function changeScope(id: string) {
    if (draft !== (insight?.current.content ?? "") && !window.confirm("Discard the unsaved insights draft when changing scope?")) return;
    setScope(id); void perform(() => loadInsight(id));
  }
  return <div className="space-y-4 text-zinc-200">
    <header><h1 className="text-xl font-medium text-white">Memories & Insights</h1></header>
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
    {tab === "memories" ? <>
      <form className={`${panel} flex flex-wrap items-end gap-3`} onSubmit={(event) => { event.preventDefault(); void perform(() => load()); }}>
        <label>Kind<select aria-label="Memory kind" className={`${input} ml-2 cursor-pointer`} value={kind} onChange={(event) => setKind(event.target.value)}><option value="">All</option>{["research", "decision", "review"].map((value) => <option key={value}>{value}</option>)}</select></label>
        <label>Instrument<input className={`${input} ml-2 w-24`} maxLength={32} value={coin} onChange={(event) => setCoin(event.target.value)} /></label><label>Topic<input className={`${input} ml-2 w-40`} maxLength={64} value={topic} onChange={(event) => setTopic(event.target.value)} /></label>
        <label>Producer<select className={`${input} ml-2 cursor-pointer`} value={producer} onChange={(event) => setProducer(event.target.value)}><option value="">All</option>{workers.map((worker) => <option key={worker.id} value={worker.id}>{worker.name}{worker.archived ? " (archived)" : ""}</option>)}</select></label>
        <label>Status<select className={`${input} ml-2 cursor-pointer`} value={status} onChange={(event) => setStatus(event.target.value)}><option value="">All</option>{["fresh", "stale", "retracted", "invalid_evidence", "durable"].map((value) => <option key={value}>{value}</option>)}</select></label>
        <label>From (UTC)<input type="date" className={`${input} ml-2`} value={start} onChange={(event) => setStart(event.target.value)} /></label><label>Until exclusive (UTC)<input type="date" className={`${input} ml-2`} value={end} onChange={(event) => setEnd(event.target.value)} /></label>
        <input aria-label="Search memory summaries" placeholder="Search summaries" className={input} maxLength={200} value={query} onChange={(event) => setQuery(event.target.value)} /><button className={button} disabled={busy}>Search / newest</button>
      </form>
      <div className="grid gap-4 lg:grid-cols-2"><section className={panel} aria-label="Memory publications">{!items.length && <p>No publications yet. Configure an analysis/review worker and its selected memory tools.</p>}{items.map((item) => <button key={item.ref} disabled={busy} onClick={() => { void perform(() => open(item.ref)); }} className={`${button} block w-full space-y-1 text-left`}><strong>{item.ref} · {item.kind} · {item.instrument ?? "Agent-wide"} · {item.status}</strong><span className="block">{item.summary}</span><span className="block text-xs text-zinc-400">{item.topic} · {item.interval ?? "—"} · {workers.find((worker) => worker.id === item.producerSubagentId)?.name ?? "Root"} · {new Date(item.createdAt).toISOString()}</span></button>)}<button className={button} disabled={busy || !next} onClick={() => { if (next) void perform(() => load(next)); }}>Next page</button></section>
      {selected && <section className={panel} aria-label="Memory detail"><h2 className="text-lg">{selected.ref} · {selected.kind} · {selected.status}</h2><button className={button} onClick={() => { void perform(() => navigator.clipboard.writeText(selected.ref)); }}>Copy short ref</button><p>{selected.summary}</p><pre className="whitespace-pre-wrap break-words text-sm">{selected.content}</pre>{Object.keys(selected.payload).length > 0 ? <pre aria-label="Additional memory data" className="whitespace-pre-wrap break-words text-xs text-zinc-400">{JSON.stringify(selected.payload, null, 2)}</pre> : null}<p className="text-xs">Evidence boundary: {selected.evidenceAsOf === null ? "None" : new Date(selected.evidenceAsOf).toISOString()} · Valid until: {selected.validUntil === null ? "No market expiry" : new Date(selected.validUntil).toISOString()}</p><p className="text-xs">{selected.sourceRunId ? <a className="cursor-pointer text-violet-300" href={`/agents/${agentId}/sub-agents/${encodeURIComponent(selected.producerSubagentId!)}/runs/${encodeURIComponent(selected.sourceRunId)}`}>Source run</a> : "Root authored"}</p><div className="flex flex-wrap gap-2">{selected.links.map((link) => <button key={`${link.ref}:${link.relation}`} className={button} disabled={busy} onClick={() => { void perform(() => open(link.ref)); }}>{link.relation} {link.ref}</button>)}</div>{selected.artifactIds.map((id) => <button type="button" className="block cursor-pointer text-left text-sm text-violet-300" key={id} onClick={() => setArtifactId(id)}>Supporting artifact {id}</button>)}</section>}</div>
    </> : <section className={panel}>
      <label>Insights scope<select aria-label="Insights scope" className={`${input} ml-3 cursor-pointer`} value={scope} disabled={busy} onChange={(event) => changeScope(event.target.value)}><option value="">Agent-wide</option>{workers.filter((worker) => worker.kind === "worker").map((worker) => <option value={worker.id} key={worker.id}>{worker.name}{worker.archived ? " (archived history)" : ""}</option>)}</select></label>
      {insight && <><h2>Current v{insight.current.version} · {insight.current.version === 0 ? "Empty document" : "Immutable revision head"}</h2><pre className="whitespace-pre-wrap break-words text-sm">{insight.current.content || "No insights yet. Every new assignment still receives an explicit empty v0 snapshot."}</pre>
        {insight.head && <p className="text-xs text-zinc-400">Updated {new Date(insight.head.createdAt).toISOString()} · {insight.head.userId ? "Operator" : "Reviewer"} · {insight.head.reason} · evidence: {insight.head.evidenceRefs.join(", ") || "None"}</p>}
        {insight.revision && <RevisionView snapshot={insight.revision} onCopy={() => { setDraft(insight.revision!.content); setRefs(insight.revision!.evidenceRefs.join(", ")); setReason(`Copy of historical v${insight.revision!.version}`); }} agentId={agentId} />}
        <details open><summary className="cursor-pointer">Revision history</summary><div className="mt-3 flex flex-wrap gap-2">{insight.history?.items.map((item) => <button key={item.version} disabled={busy} className={button} onClick={() => { void perform(() => loadInsight(scope, item.version, undefined, true)); }}>v{item.version} · {new Date(item.createdAt).toISOString()} · {item.reason}</button>)}{insight.history?.nextBeforeVersion && <button className={button} disabled={busy} onClick={() => { void perform(() => loadInsight(scope, undefined, insight.history!.nextBeforeVersion!, true)); }}>Older revisions</button>}</div></details>
        <label className="block">Replacement draft · based on v{base}<textarea aria-label="Insights document draft" rows={10} maxLength={8000} className={`${input} mt-2 block w-full`} value={draft} onChange={(event) => setDraft(event.target.value)} /></label>
        <label className="block">Reason<input className={`${input} mt-2 block w-full`} maxLength={1000} value={reason} onChange={(event) => setReason(event.target.value)} /></label><label className="block">Evidence refs (comma separated)<input className={`${input} mt-2 block w-full`} value={refs} maxLength={400} onChange={(event) => setRefs(event.target.value)} /></label>
        <div className="flex flex-wrap gap-2"><button disabled={busy || !reason.trim()} className={button} onClick={() => { void perform(save); }}>Save new revision</button><button className={button} disabled={busy} onClick={() => { if (window.confirm("Clear the draft? Saving creates an audited empty revision; history remains.")) setDraft(""); }}>Clear draft</button><button className={button} disabled={busy} onClick={() => { void perform(async () => { await loadInsight(scope, undefined, undefined, true); const latest = await getInsights({ data: { agentId, scope: insightScope() } }); if (!latest.ok) throw new Error(latest.error); if (window.confirm(`Reconcile your preserved draft against latest v${latest.data.current.version}? Its current content is displayed above. Saving will replace that version.`)) setBase(latest.data.current.version); }); }}>Refresh / reconcile (preserve draft)</button></div>
        <p className="text-xs text-zinc-400">Full replacement uses compare-and-swap. A conflict keeps your draft. Copying historical content creates a new revision, never a rewind. Guidance cannot edit permissions or enforced risk controls. Running/queued assignments retain their original snapshots.</p>
      </>}
    </section>}
    {artifactId && <ArtifactPreviewDialog key={`${agentId}:${artifactId}`} agentId={agentId} artifactId={artifactId} onClose={() => setArtifactId(null)} />}
  </div>;
}
function RevisionView({ snapshot, onCopy, agentId }: { snapshot: InsightSnapshot & { reason: string; createdAt: number; userId: string | null; sourceRunId: string | null; producerSubagentId: string | null; evidenceRefs: string[] }; onCopy: () => void; agentId: string }) {
  return <article className="space-y-3 rounded-lg border border-zinc-700 p-4"><h3>Revision v{snapshot.version} · {new Date(snapshot.createdAt).toISOString()}</h3><p>{snapshot.reason} · {snapshot.userId ? "Operator" : "Reviewer"}{snapshot.sourceRunId && <a className="ml-2 cursor-pointer text-violet-300" href={`/agents/${agentId}/sub-agents/${encodeURIComponent(snapshot.producerSubagentId!)}/runs/${encodeURIComponent(snapshot.sourceRunId)}`}>Source run</a>}</p><pre className="whitespace-pre-wrap break-words text-sm">{snapshot.content || "Audited clear"}</pre><div>{snapshot.evidenceRefs.map((ref) => <a className="mr-3 cursor-pointer text-violet-300" key={ref} href={`?memory=${ref}`}>{ref}</a>)}</div><button type="button" className={button} onClick={onCopy}>Copy content to new draft</button></article>;
}

import { Link } from "@tanstack/react-router";
import { useState, type ReactNode } from "react";
import { InstrumentLabel } from "./instrument-label.js";
import type { MemoryDetail, MemoryRelation } from "../memory/memory-contracts.js";
import { ArtifactPreviewDialog } from "./artifact-preview-dialog.js";
import { MemoryStatus } from "./memory-status.js";
import { MemoryBadge } from "./memory-badge.js";
import { ToggleTimestamp } from "./toggle-timestamp.js";

const relationLabels: Record<MemoryRelation, string> = { based_on: "Based on", corrects: "Corrects", supersedes: "Supersedes", reviews: "Reviews" };
const link = "cursor-pointer text-violet-300 hover:text-violet-200 hover:underline";

export function AgentMemoryDetailPage({ agentId, memory }: { agentId: string; memory: MemoryDetail }) {
  const [artifactId, setArtifactId] = useState<string | null>(null);
  const [error, setError] = useState("");
  async function copyRef() {
    try { await navigator.clipboard.writeText(memory.ref); setError(""); }
    catch { setError("Unable to copy memory reference"); }
  }
  return <section className="w-full min-w-0 space-y-4 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6 text-zinc-200" aria-label="Memory detail">
    <header className="space-y-2">
      <nav aria-label="Memory breadcrumb" className="flex min-w-0 flex-wrap items-center gap-2">
        <Link to="/agents/$agentId/memories" params={{ agentId }} search={{}} className="cursor-pointer text-sm text-zinc-400 hover:text-white"><span aria-hidden="true">← </span>Memories</Link>
        <span aria-hidden="true" className="text-zinc-600">/</span>
        <h1 aria-current="page" className="break-words text-lg font-medium text-white">Memory {memory.ref}</h1>
        <button type="button" aria-label="Copy memory reference" title="Copy memory reference" className="cursor-pointer rounded-md p-1.5 text-zinc-400 hover:bg-zinc-800 hover:text-white" onClick={() => { void copyRef(); }}>
          <svg aria-hidden="true" className="size-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"><rect x="9" y="9" width="12" height="12" rx="2" /><path d="M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1" /></svg>
        </button>
        <MemoryStatus status={memory.status} />
      </nav>
    </header>
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
    <dl aria-label="Memory stats" className="grid grid-cols-1 gap-x-6 gap-y-5 border-t border-zinc-800 pt-5 sm:grid-cols-2 xl:grid-cols-4">
      <MemoryStat label="Kind"><MemoryBadge value={memory.kind} /></MemoryStat>
      <MemoryStat label="Instrument">{memory.instrument ? <InstrumentLabel symbol={memory.instrument} /> : "Agent-wide"}</MemoryStat>
      <MemoryStat label="Topic"><MemoryBadge value={memory.topic} /></MemoryStat>
      <MemoryStat label="Interval">{memory.interval ?? "—"}</MemoryStat>
      <MemoryStat label="Created"><ToggleTimestamp timestamp={memory.createdAt} /></MemoryStat>
      <MemoryStat label="Evidence boundary">{memory.evidenceAsOf === null ? "None" : <MemoryTimestamp value={memory.evidenceAsOf} />}</MemoryStat>
      <MemoryStat label="Valid until">{memory.validUntil === null ? "No market expiry" : <ToggleTimestamp timestamp={memory.validUntil} label="expiry" />}</MemoryStat>
      <MemoryStat label="Producer">{memory.producerSubagentId ? <Link className={link} to="/agents/$agentId/sub-agents/$subagentId" params={{ agentId, subagentId: memory.producerSubagentId }}>{memory.producerSubagentId}</Link> : "Root agent"}</MemoryStat>
      <MemoryStat label="Source">{memory.sourceRunId && memory.producerSubagentId ? <Link className={link} to="/agents/$agentId/sub-agents/$subagentId/runs/$runId" params={{ agentId, subagentId: memory.producerSubagentId, runId: memory.sourceRunId }}>Source run</Link> : "Root authored"}</MemoryStat>
      {memory.links.length > 0 && <div className="min-w-0 sm:col-span-2 xl:col-span-4"><dt className="text-xs uppercase tracking-[0.16em] text-zinc-500">Related memories</dt><dd className="mt-2 flex flex-wrap gap-x-4 gap-y-2 text-sm">{memory.links.map((related) => <Link key={`${related.ref}:${related.relation}`} className={link} to="/agents/$agentId/memories/$memoryRef" params={{ agentId, memoryRef: related.ref }}>{relationLabels[related.relation]}: {related.ref}</Link>)}</dd></div>}
    </dl>
    <section className="space-y-4 border-t border-zinc-800 pt-4" aria-label="Memory contents">
      <p>{memory.summary}</p>
      <pre className="whitespace-pre-wrap break-words text-sm">{memory.content}</pre>
      {Object.keys(memory.payload).length > 0 ? <pre aria-label="Additional memory data" className="whitespace-pre-wrap break-words text-xs text-zinc-400">{JSON.stringify(memory.payload, null, 2)}</pre> : null}
    </section>
    {memory.artifactIds.length > 0 && <section className="space-y-2"><h2 className="text-sm font-medium text-white">Supporting artifacts</h2>{memory.artifactIds.map((id) => <button type="button" className="block cursor-pointer text-left text-sm text-violet-300" key={id} onClick={() => setArtifactId(id)}>Supporting artifact {id}</button>)}</section>}
    {artifactId && <ArtifactPreviewDialog key={`${agentId}:${artifactId}`} agentId={agentId} artifactId={artifactId} onClose={() => setArtifactId(null)} />}
  </section>;
}

function MemoryStat({ label, children }: { label: string; children: ReactNode }) {
  return <div className="min-w-0"><dt className="text-xs uppercase tracking-[0.16em] text-zinc-500">{label}</dt><dd className="mt-2 break-words text-sm text-zinc-200">{children}</dd></div>;
}

function MemoryTimestamp({ value }: { value: number }) {
  const iso = new Date(value).toISOString();
  return <time dateTime={iso} title={iso} className="font-mono text-xs">{iso.replace("T", " ").slice(0, 19)} UTC</time>;
}

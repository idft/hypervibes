import { useRef, useState } from "react";
import { listArtifacts, readArtifact, changeArtifact } from "../web/artifacts.js";
import { ArtifactContent, artifactContentUrl } from "./artifact-content.js";
import type { ArtifactMeta } from "../research/artifacts.js";

type Page = { items: ArtifactMeta[]; nextOffset: number | null };
type Preview = Awaited<ReturnType<import("../research/artifacts.js").ArtifactService["read"]>>;
const button = "cursor-pointer rounded-lg border border-zinc-700 px-3 py-2 text-sm hover:bg-zinc-800 disabled:cursor-wait disabled:opacity-50";
export function AgentArtifactsPage({ agentId, initial, initialArtifact, initialError }: { agentId: string; initial: Page; initialArtifact?: Preview | null; initialError?: string }) {
  const [page, setPage] = useState(initial), [selected, setSelected] = useState<Preview | null>(initialArtifact ?? null);
  const [query, setQuery] = useState(""), [error, setError] = useState(initialError ?? ""), [busy, setBusy] = useState(false);
  const flight = useRef(false);
  async function perform(action: () => Promise<void>) {
    if (flight.current) return;
    flight.current = true; setBusy(true); setError("");
    try { await action(); } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to load artifacts"); }
    finally { flight.current = false; setBusy(false); }
  }
  async function load(offset = 0) {
    const result = await listArtifacts({ data: { agentId, offset, query } });
    if (!result.ok) throw new Error(result.error);
    setPage(result.data);
  }
  async function open(id: string, offset = 0) {
    const result = await readArtifact({ data: { agentId, id, offset } });
    if (!result.ok) throw new Error(result.error);
    setSelected(result.data);
  }
  async function change(action: string) {
    if (!selected) return;
    if (action === "delete" && !window.confirm("Permanently delete this artifact? Referenced attachments cannot be deleted until their report is removed.")) return;
    const result = await changeArtifact({ data: { agentId, id: selected.id, action } });
    if (!result.ok) throw new Error(result.error);
    if (action === "delete") setSelected(null); else await open(selected.id);
    await load();
  }
  return <div className="space-y-4 p-6">
    <header className="space-y-2"><h1 className="text-xl font-medium text-white">Artifacts</h1><p className="text-sm text-zinc-400">Shared by this root Agent and all its sub-agents. 5 MiB per item · 100 MiB per Agent. Temporary items expire after seven days; reports protect their attachments. Memory is a separate future feature.</p></header>
    <form className="flex gap-2" onSubmit={(event) => { event.preventDefault(); void perform(() => load()); }}><input aria-label="Search artifact titles and descriptions" className="min-w-0 flex-1 rounded-lg border border-zinc-700 bg-zinc-950 px-3 py-2 text-sm" maxLength={160} value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search titles and descriptions" /><button disabled={busy} className={button}>Search / Refresh</button></form>
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
    <div className="grid gap-4 xl:grid-cols-[20rem_1fr]">
      <section className="space-y-2" aria-label="Artifact library">{page.items.map((item) => <button type="button" disabled={busy} key={item.id} className={`${button} block w-full space-y-1 text-left ${selected?.id === item.id ? "border-violet-500" : ""}`} onClick={() => { void perform(() => open(item.id)); }}><span className="block font-medium">{item.title}</span><span className="block text-xs text-zinc-400">{item.mediaType} · {item.size.toLocaleString()} bytes · {item.expiresAt === null ? "Retained" : "Temporary"}</span><span className="block text-xs text-zinc-500">{[item.instrument, item.interval, ...item.tags].filter(Boolean).join(" · ")}</span></button>)}{!page.items.length && <p className="text-sm text-zinc-400">No artifacts. Enable artifact_create on a research sub-agent to save reports and attachments.</p>}
      <div className="flex gap-2"><button className={button} disabled={busy} onClick={() => { void perform(() => load()); }}>First page</button>{page.nextOffset !== null && <button className={button} disabled={busy} onClick={() => { void perform(() => load(page.nextOffset!)); }}>Next page</button>}</div></section>
      {selected && <article className="min-w-0 space-y-4 rounded-xl border border-zinc-800 p-4"><h2 className="text-lg text-white">{selected.title}</h2><p className="text-sm text-zinc-400">{selected.description}</p><p className="text-xs text-zinc-500">Created {new Date(selected.createdAt).toISOString()} · {selected.expiresAt === null ? "Retained" : `Own expiry ${new Date(selected.expiresAt).toISOString()} (referencing reports may protect it)`} · Conversation {selected.conversationId ?? "—"} · Task {selected.taskId ?? "—"}</p>
      <div className="flex flex-wrap gap-2"><a className={button} href={artifactContentUrl(agentId, selected.id)}>Download</a><button disabled={busy} className={button} onClick={() => { void perform(() => change(selected.expiresAt === null ? "expire" : "retain")); }}>{selected.expiresAt === null ? "Expire in seven days" : "Keep / Pin"}</button><button disabled={busy} className={`${button} text-red-300`} onClick={() => { void perform(() => change("delete")); }}>Delete</button></div>
      {selected.mediaType.startsWith("image/") ? <img className="max-h-[40rem] max-w-full object-contain" src={artifactContentUrl(agentId, selected.id, true)} alt={selected.title} /> : <ArtifactContent agentId={agentId} content={selected.content ?? ""} attachments={selected.attachments} />}
      {selected.nextOffset !== null && <button disabled={busy} className={button} onClick={() => { void perform(() => open(selected.id, selected.nextOffset!)); }}>Next text portion</button>}
      {selected.attachments.length > 0 && <section className="space-y-2"><h3 className="text-sm text-white">Attachments</h3>{selected.attachments.map((item) => <button disabled={busy} className={`${button} mr-2`} key={item.id} onClick={() => { void perform(() => open(item.id)); }}>{item.title}</button>)}</section>}
      </article>}
    </div>
  </div>;
}

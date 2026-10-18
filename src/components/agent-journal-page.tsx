import { useEffect, useRef, useState } from "react";
import type { JournalTrade } from "../hyperliquid/journal-model.js";
import type { JournalNote, JournalPage } from "../hyperliquid/journal-service.js";
import { amount } from "../hyperliquid/journal-decimal.js";
import { compareDecimals, formatMoneyText } from "../hyperliquid/decimal.js";
import { addJournalNote, getAgentJournal, getJournalAudit, getJournalNotes } from "../web/journal.js";

import { InstrumentLabel } from "./instrument-label.js";

import type { FillAttribution } from "../memory/memory-contracts.js";
interface NoteTarget { id: string; label: string; trade?: JournalTrade }
const button = "cursor-pointer rounded-full border border-zinc-800 px-4 py-1.5 text-sm text-zinc-300 hover:border-zinc-600 hover:text-white disabled:cursor-not-allowed disabled:opacity-40";
const cell = "whitespace-nowrap px-3 py-3";
function Money({ value, color = false, missing = "Incomplete" }: { value: string | null; color?: boolean; missing?: string }) {
  const sign = value === null ? 0 : compareDecimals(value, "0");
  return <span className={`font-mono text-xs tabular-nums ${value === null ? "text-amber-300" : color && sign ? sign < 0 ? "text-red-400" : "text-emerald-400" : ""}`}>{value === null ? missing : formatMoneyText(value)}</span>;
}
function price(notional: string, size: string) { return amount(size).zero ? "—" : formatMoneyText(amount(notional).div(amount(size)).toString()); }
function Time({ value }: { value: number }) { return <time dateTime={new Date(value).toISOString()}>{new Date(value).toISOString().replace("T", " ").slice(0, 19)} UTC</time>; }

export function AgentJournalPage({ id, initial, onNavigate }: { id: string; initial: JournalPage; onNavigate: (view: "trades" | "activity", page: number) => void }) {
  const [data, setData] = useState(initial);
  const [error, setError] = useState("");
  const [target, setTarget] = useState<NoteTarget | null>(null);
  useEffect(() => {
    setData(initial); setError("");
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const refresh = async () => {
      try {
        if (!document.hidden) {
          const result = await getAgentJournal({ data: { id, view: initial.view, page: String(initial.page) } });
          if (!cancelled) { if (result.ok) { setData(result.data); setError(""); } else setError(result.error); }
        }
      } catch { if (!cancelled) setError("Unable to refresh the journal. Displayed data may be stale."); }
      if (!cancelled) timer = setTimeout(() => { void refresh(); }, 5_000);
    };
    if (initial.linked) timer = setTimeout(() => { void refresh(); }, 5_000);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [id, initial]);
  const notes = (entryId: string, label: string, count: number) => <button type="button" className="cursor-pointer text-violet-300 hover:text-violet-200" aria-label={`${count ? "View" : "Add"} ${label} notes`} onClick={() => setTarget({ id: entryId, label, trade: data.trades.find((t) => t.id === entryId) })}>✎{count > 0 && <span className="ml-1 text-xs">{count}</span>}</button>;
  const syncing = data.pending || data.sync.length < 3 || data.sync.some((s) => s.updated_at === null);
  return <section className="flex min-h-0 w-full flex-col rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
    <div className="flex items-center justify-between gap-4"><h2 className="text-lg font-medium text-white">Transactions</h2><div className="flex items-center gap-2"><button className={button} disabled={data.page <= 1} onClick={() => onNavigate(data.view, data.page - 1)}>Previous</button><span className="text-xs text-zinc-500">Page {data.page} of {data.pages}</span><button className={button} disabled={data.page >= data.pages} onClick={() => onNavigate(data.view, data.page + 1)}>Next</button></div></div>
    <nav aria-label="Transaction view" className="mt-4 flex gap-2">{(["trades", "activity"] as const).map((view) => <button key={view} className={`${button} ${data.view === view ? "border-violet-500 text-white" : ""}`} aria-pressed={data.view === view} onClick={() => onNavigate(view, 1)}>{view === "trades" ? "Trades" : "Activity"}</button>)}</nav>
    {!data.linked ? <p className="mt-4 text-sm text-zinc-400">No trading account is linked to this Agent.</p> : <>
      {data.historyLimited && <p role="status" className="mt-4 text-sm text-amber-200">The initial import reached Hyperliquid’s 10,000-fill history limit. Earlier activity may be missing; account balances are incomplete.</p>}
      {syncing && <p role="status" className="mt-4 text-sm text-amber-200">Synchronizing account history… Totals may be incomplete.</p>}
      {data.sync.filter((s) => s.error).map((s) => <p role="alert" key={s.stream} className="mt-3 text-sm text-amber-200">{s.stream}: {s.error}</p>)}
      {error && <p role="alert" className="mt-3 text-sm text-red-300">{error}</p>}
      {data.total === 0 ? <p className="mt-6 text-sm text-zinc-400">{data.view === "trades" ? "No trades yet." : "No transactions have synced yet."}</p> : <div className="mt-4 min-h-0 overflow-auto"><table className="w-full text-sm text-zinc-300"><thead className="text-left text-xs uppercase text-zinc-500"><tr>{(data.view === "trades" ? ["Trade", "Status", "Notes", "Last activity", "Entry / Exit price", "Amount", "Realized P&L", "USDC fees", "Funding", "Net realized"] : ["Time", "Notes", "Type", "Symbol/Asset", "Fee USDC", "Realized P&L USDC", "USDC delta", "Balance"]).map((label) => <th key={label} className={cell}>{label}</th>)}</tr></thead><tbody className="divide-y divide-zinc-800">
        {data.view === "trades" ? data.trades.map((trade) => <tr key={trade.id} className="hover:bg-zinc-900/50">
          <td className={cell}><button className="inline-flex cursor-pointer items-center gap-1 text-violet-300" onClick={() => setTarget({ id: trade.id, label: `${trade.symbol} ${trade.direction} trade`, trade })}><InstrumentLabel symbol={trade.symbol} /> · {trade.direction}</button></td>
          <td className={cell}>{trade.status}{trade.coverageReason && <span title={trade.coverageReason} className="block text-xs text-amber-300">Incomplete coverage</span>}</td><td className={cell}>{notes(trade.id, `${trade.symbol} trade`, trade.noteCount)}</td><td className={`${cell} text-xs text-zinc-400`}><Time value={trade.lastActivityAt} /></td>
          <td className={`${cell} font-mono text-xs`}>{price(trade.entryNotional, trade.entrySize)} / {price(trade.exitNotional, trade.exitSize)}</td><td className={`${cell} font-mono text-xs`}>{trade.entrySize}</td>
          <td className={cell}><Money value={trade.grossPnl} color /></td><td className={cell}><Money value={trade.feeCoverage ? trade.fees : null} /></td><td className={cell}><Money value={trade.funding} color /></td><td className={cell}><Money value={trade.net} color /></td>
        </tr>) : data.activity.map((event) => <tr key={event.id} className="hover:bg-zinc-900/50"><td className={`${cell} text-xs`}><Time value={event.time} /></td><td className={cell}>{notes(event.id, "transaction", event.noteCount)}</td><td className={cell}>{event.type === "fill A" ? "Fill · Sell" : event.type === "fill B" ? "Fill · Buy" : event.type}</td><td className={cell}>{event.symbol && (event.fill || event.kind === "funding") ? <InstrumentLabel symbol={event.symbol} /> : event.symbol ?? "—"}</td><td className={cell}><Money value={event.fee} missing="—" /></td><td className={cell}><Money value={event.pnl} color missing="—" /></td><td className={cell}><Money value={event.delta} color /></td><td className={cell}><Money value={event.balance} /></td></tr>)}
      </tbody></table></div>}
      <p className="mt-4 text-xs text-zinc-500">{data.total} {data.view === "trades" ? "trades" : "events"} · Automatic flat-to-flat grouping for perpetuals. Spot fills appear in Activity.</p>
      <p className="mt-2 text-xs text-zinc-500">Hyperliquid provides only the latest 10,000 historical fills. Imported history is retained locally. Funding and balances reflect imported events, not a verified account-equity snapshot. Unknown cash impacts remain incomplete.</p>
    </>}
    {target && <NotesModal key={target.id} id={id} target={target} onClose={() => setTarget(null)} />}
  </section>;
}

function NotesModal({ id, target, onClose }: { id: string; target: NoteTarget; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [notes, setNotes] = useState<JournalNote[]>([]);
  const [audit, setAudit] = useState<{ items: FillAttribution[]; truncated: boolean; coverage: string } | null>(null);
  const [auditError, setAuditError] = useState("");
  const [loading, setLoading] = useState(true);
  const [body, setBody] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    dialog.current?.showModal();
    let cancelled = false;
    void getJournalNotes({ data: { id, target: target.id } }).then((result) => {
      if (cancelled) return;
      if (result.ok) setNotes(result.data); else setError(result.error);
    }).catch(() => { if (!cancelled) setError("Unable to load notes."); }).finally(() => { if (!cancelled) setLoading(false); });
    void getJournalAudit({ data: { id, target: target.id } }).then((result) => { if (!cancelled) { if (result.ok) setAudit(result.data); else setAuditError(result.error); } }).catch(() => { if (!cancelled) setAuditError("Decision attribution is unavailable; do not infer that these fills had no rationale."); });
    return () => { cancelled = true; };
  }, [id, target.id]);
  const submit = async () => {
    if (busy) return; setBusy(true); setError("");
    try {
      const result = await addJournalNote({ data: { id, target: target.id, body } });
      if (!result.ok) { setError(result.error); return; }
      setBody("");
      const refreshed = await getJournalNotes({ data: { id, target: target.id } });
      if (refreshed.ok) setNotes(refreshed.data); else setError(refreshed.error);
    } catch { setError("Unable to save or refresh notes. Check the journal before retrying."); }
    finally { setBusy(false); }
  };
  return <dialog ref={dialog} aria-labelledby="journal-notes-title" onCancel={onClose} className="m-auto max-h-[85vh] w-[calc(100vw-2rem)] max-w-3xl overflow-auto rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6 text-zinc-200 backdrop:bg-black/70">
    <div className="flex items-center justify-between"><h3 id="journal-notes-title" className="text-lg font-medium">{target.label} notes</h3><button className={button} aria-label="Close notes" onClick={onClose}>Close</button></div>
    {target.trade && <details className="mt-4 rounded-xl border border-zinc-800 p-4"><summary className="cursor-pointer text-sm">Trade fills</summary><div className="mt-3 space-y-2 text-xs text-zinc-400">{target.trade.parts.map((part) => <p key={`${part.eventId}-${part.segment}`}><Time value={part.time} /> · {part.side} · {part.segment} {part.quantity} @ {formatMoneyText(part.price)} · fee {part.fee ?? "unknown"} · realized <Money value={part.pnl} color /></p>)}</div></details>}
    <section className="mt-4 space-y-2 rounded-xl border border-zinc-800 p-4" aria-label="Decision attribution"><h4 className="text-sm">Decision & order attribution</h4>{auditError && <p role="status" className="text-xs text-amber-200">{auditError}</p>}{audit ? <><p className="text-xs text-zinc-400">{audit.coverage}{audit.truncated ? " First 50 fills only; inspect the bounded reviewer audit for more." : ""}</p>{!audit.items.length && <p className="text-xs text-zinc-400">No applicable fill attribution for this entry.</p>}{audit.items.map((item) => <p key={item.fillId} className="text-xs text-zinc-300">OID {item.exchangeOid ?? "unknown"} · {item.origin} · {item.requestId ? `request ${item.requestId}` : "external/unmatched"} · {item.decisionRef ? <a className="cursor-pointer text-violet-300" href={`/agents/${encodeURIComponent(id)}/memories?memory=${item.decisionRef}`}>{item.decisionRef}</a> : item.origin === "legacy/unattributed" ? "legacy/unattributed" : "No decision link (management may omit)"}</p>)}</> : !auditError && <p className="text-xs">Loading attribution…</p>}</section>
    <div className="my-4 space-y-3">{loading ? <p role="status">Loading notes…</p> : notes.length === 0 ? <p className="text-sm text-zinc-500">No notes yet.</p> : notes.map((note) => <article key={note.id} className="rounded-xl border border-zinc-800 p-4"><p className="whitespace-pre-wrap break-words text-sm">{note.body}</p><p className="mt-2 text-xs text-zinc-500">human · {note.authorId} · <Time value={note.createdAt} /></p></article>)}</div>
    {error && <p role="alert" className="my-3 text-sm text-red-300">{error}</p>}
    <form onSubmit={(event) => { event.preventDefault(); void submit(); }}><label htmlFor="journal-note" className="mb-2 block text-sm">Add a note</label><textarea id="journal-note" value={body} onChange={(event) => setBody(event.target.value)} rows={3} maxLength={4000} required className="w-full rounded-lg border border-zinc-700 bg-zinc-950 p-3 text-sm" /><div className="mt-3 flex justify-end"><button className={button} disabled={busy || !body.trim()}>{busy ? "Saving…" : "Add note"}</button></div></form>
  </dialog>;
}

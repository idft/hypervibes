import { useEffect, useId, useState } from "react";
import { getChartInstruments } from "../web/charts.js";
import { ConfigDialog, configButton, configInput } from "./config-dialog.js";
import { InstrumentLabel } from "./instrument-label.js";

// Matches the risk settings service's allowlist validation limit.
const maxSelections = 50;
const radio = "size-4 shrink-0 cursor-pointer appearance-none rounded-full border border-zinc-600 bg-zinc-950 checked:border-[5px] checked:border-violet-500 checked:bg-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400";

export function InstrumentPicker({ agentId, value, onChange, disabled = false }: { agentId: string; value: string[] | null; onChange: (value: string[] | null) => void; disabled?: boolean }) {
  const [open, setOpen] = useState(false);
  return <section aria-label="Allowed instruments" className="space-y-2 text-sm text-zinc-300">
    <h3>Allowed instruments</h3>
    <div className="flex flex-wrap items-center gap-3 rounded-2xl border border-zinc-800 bg-zinc-950/60 p-4">
      <div className="flex min-w-0 flex-1 flex-wrap gap-2">
        {value === null ? <span className="self-center text-zinc-400">All instruments · No limit</span> : value.map((symbol) => <span key={symbol} className="rounded-xl border border-zinc-800 bg-zinc-950/70 px-3 py-2"><InstrumentLabel symbol={symbol} /></span>)}
      </div>
      <button type="button" disabled={disabled} aria-haspopup="dialog" onClick={() => setOpen(true)} className={`${configButton} shrink-0`}>Select instruments</button>
    </div>
    {open && <InstrumentDialog agentId={agentId} value={value} onCancel={() => setOpen(false)} onApply={(next) => { onChange(next); setOpen(false); }} />}
  </section>;
}

function InstrumentDialog({ agentId, value, onCancel, onApply }: { agentId: string; value: string[] | null; onCancel: () => void; onApply: (value: string[] | null) => void }) {
  const name = useId();
  const [unlimited, setUnlimited] = useState(value === null);
  const [selected, setSelected] = useState(() => new Set(value ?? []));
  const [search, setSearch] = useState("");
  const [catalog, setCatalog] = useState<string[] | null>(null);
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let disposed = false;
    setCatalog(null); setError("");
    // Charts already owns an authenticated, cached catalog of active main-DEX perps.
    void getChartInstruments({ data: { id: agentId } }).then((result) => {
      if (disposed) return;
      if (!result.ok) throw new Error(result.error);
      setCatalog(result.data);
    }).catch((cause: unknown) => {
      if (!disposed) setError(cause instanceof Error ? cause.message : "Unable to load instruments.");
    });
    return () => { disposed = true; };
  }, [agentId, attempt]);
  const active = new Set(catalog ?? []);
  // Keep saved symbols visible if the venue delists them; never silently drop restrictions.
  const symbols = [...new Set([...(catalog ?? []), ...(value ?? [])])].sort();
  const matches = symbols.filter((symbol) => symbol.toLowerCase().includes(search.trim().toLowerCase()));
  const selectable = matches.filter((symbol) => active.has(symbol));
  const bulk = new Set([...selected, ...selectable]);
  const ready = catalog !== null;
  function toggle(symbol: string) {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(symbol)) next.delete(symbol);
      else if (active.has(symbol) && next.size < maxSelections) next.add(symbol);
      return next;
    });
  }
  return <ConfigDialog title="Select trading instruments" onCancel={onCancel} onApply={() => {
    if (unlimited) onApply(null);
    else if (ready && selected.size > 0 && selected.size <= maxSelections) onApply([...selected].sort());
  }} applyDisabled={!unlimited && (!ready || selected.size === 0 || selected.size > maxSelections)}>
    <fieldset className="flex shrink-0 flex-wrap gap-4 text-sm">
      <legend className="sr-only">Instrument restrictions</legend>
      <label className="flex cursor-pointer items-center gap-2"><input type="radio" name={name} className={radio} checked={unlimited} onChange={() => setUnlimited(true)} />No limit (all instruments)</label>
      <label className="flex cursor-pointer items-center gap-2"><input type="radio" name={name} className={radio} checked={!unlimited} onChange={() => setUnlimited(false)} />Selected instruments only</label>
    </fieldset>
    <label className="block shrink-0 space-y-2 text-sm">Search instruments<input autoFocus type="search" value={search} onChange={(event) => setSearch(event.target.value)} className={configInput} placeholder="Filter instruments" autoComplete="off" /></label>
    <div className="flex shrink-0 flex-wrap items-center gap-2">
      <button type="button" disabled={unlimited || !ready || !selectable.length || bulk.size > maxSelections} onClick={() => setSelected(bulk)} className={configButton}>{search.trim() ? "Select all shown" : "Select all"}</button>
      <button type="button" disabled={unlimited || !selected.size} onClick={() => setSelected(new Set())} className={configButton}>Select none</button>
      <span className="ml-auto text-xs text-zinc-400">{unlimited ? "All instruments allowed" : `${selected.size} / ${maxSelections} selected`}</span>
    </div>
    {!unlimited && !selected.size && <p role="status" className="text-sm text-amber-200">Select at least one instrument, or choose No limit to allow all instruments.</p>}
    {!unlimited && (catalog?.length ?? 0) > maxSelections && <p className="text-xs text-zinc-400">Choose up to {maxSelections} instruments. Filter to select all matching instruments, or use No limit to allow every instrument.</p>}
    {error ? <div role="alert" className="space-y-3 text-sm text-red-300"><p>{error}</p><button type="button" onClick={() => setAttempt((current) => current + 1)} className={configButton}>Retry</button></div>
      : !ready ? <p role="status" className="text-sm text-zinc-400">Loading instruments…</p>
      : <fieldset disabled={unlimited} aria-label="Trading instruments" className="min-h-0 space-y-2 overflow-y-auto pb-5 disabled:opacity-50" style={{ maxHeight: "24rem" }}>
        {matches.map((symbol) => <label key={symbol} className="flex cursor-pointer items-center gap-3 rounded-xl border border-zinc-800 bg-zinc-950/70 px-4 py-3 text-sm transition hover:border-zinc-700 has-checked:border-violet-500 has-checked:bg-violet-950/30 has-focus-visible:ring-2 has-focus-visible:ring-violet-400 has-disabled:cursor-not-allowed">
          <span className="relative inline-flex size-5 shrink-0">
            <input type="checkbox" aria-label={symbol} checked={selected.has(symbol)} disabled={!selected.has(symbol) && (!active.has(symbol) || selected.size >= maxSelections)} onChange={() => toggle(symbol)} className="peer size-5 cursor-pointer appearance-none rounded border border-zinc-600 bg-zinc-950 checked:border-violet-500 checked:bg-violet-600 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400 disabled:cursor-not-allowed disabled:opacity-50" />
            <svg aria-hidden="true" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth={2.5} className="pointer-events-none absolute inset-0.5 size-4 text-white opacity-0 peer-checked:opacity-100"><path strokeLinecap="round" strokeLinejoin="round" d="m4 10 4 4 8-8" /></svg>
          </span>
          <InstrumentLabel symbol={symbol} />{!active.has(symbol) && <span className="ml-auto text-xs text-amber-200">Unavailable</span>}
        </label>)}
        {!matches.length && <p className="rounded-xl border border-zinc-800 p-6 text-center text-sm text-zinc-500">{catalog.length ? "No instruments match this filter." : "No active perpetual instruments are available."}</p>}
      </fieldset>}
  </ConfigDialog>;
}

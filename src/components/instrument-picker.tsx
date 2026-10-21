import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { useQuery } from "@tanstack/react-query";
import { chartInstrumentsQueryOptions } from "../web/chart-queries.js";
import { ConfigDialog, configButton, configInput } from "./config-dialog.js";
import { InstrumentLabel } from "./instrument-label.js";

// Matches the risk settings service's allowlist validation limit.
const maxSelections = 50;
const radio = "size-4 shrink-0 cursor-pointer appearance-none rounded-full border border-zinc-600 bg-zinc-950 checked:border-[5px] checked:border-violet-500 checked:bg-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400";
const instrumentRow = "flex cursor-pointer items-center gap-3 rounded-xl border border-zinc-800 bg-zinc-950/70 px-4 py-3 text-sm transition hover:border-zinc-700 has-checked:border-violet-500 has-checked:bg-violet-950/30 has-focus-visible:ring-2 has-focus-visible:ring-violet-400 has-disabled:cursor-not-allowed";

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
  const query = useQuery(chartInstrumentsQueryOptions(agentId));
  const catalog = query.data;
  const error = query.error?.message;
  const active = new Set(catalog ?? []);
  // Keep saved symbols visible if the venue delists them; never silently drop restrictions.
  const symbols = [...new Set([...(catalog ?? []), ...(value ?? [])])].sort();
  const matches = symbols.filter((symbol) => symbol.toLowerCase().includes(search.trim().toLowerCase()));
  const selectable = matches.filter((symbol) => active.has(symbol));
  const bulk = new Set([...selected, ...selectable]);
  const ready = catalog !== undefined;
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
    {error ? <div role="alert" className="space-y-3 text-sm text-red-300"><p>{error}</p><button type="button" onClick={() => { void query.refetch(); }} className={configButton}>Retry</button></div>
      : !ready ? <p role="status" className="text-sm text-zinc-400">Loading instruments…</p>
      : <fieldset disabled={unlimited} aria-label="Trading instruments" className="min-h-0 space-y-2 overflow-y-auto pb-5 disabled:opacity-50" style={{ maxHeight: "24rem" }}>
        {matches.map((symbol) => <label key={symbol} className={instrumentRow}>
          <span className="relative inline-flex size-5 shrink-0">
            <input type="checkbox" aria-label={symbol} checked={selected.has(symbol)} disabled={!selected.has(symbol) && (!active.has(symbol) || selected.size >= maxSelections)} onChange={() => toggle(symbol)} className="peer size-5 cursor-pointer appearance-none rounded border border-zinc-600 bg-zinc-950 checked:border-violet-500 checked:bg-violet-600 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400 disabled:cursor-not-allowed disabled:opacity-50" />
            <svg aria-hidden="true" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth={2.5} className="pointer-events-none absolute inset-0.5 size-4 text-white opacity-0 peer-checked:opacity-100"><path strokeLinecap="round" strokeLinejoin="round" d="m4 10 4 4 8-8" /></svg>
          </span>
          <InstrumentLabel symbol={symbol} />{!active.has(symbol) && <span className="ml-auto text-xs text-amber-200">Unavailable</span>}
        </label>)}
        {!matches.length && <p className="rounded-xl border border-zinc-800 p-6 text-center text-sm text-zinc-500">{catalog?.length ? "No instruments match this filter." : "No active perpetual instruments are available."}</p>}
      </fieldset>}
  </ConfigDialog>;
}

/** Single-selection variant using the chart's already-loaded instrument catalog. */
export function SingleInstrumentPicker({ instruments, value, onChange, disabled = false, saveError = "", className = configButton }: {
  instruments: string[]; value: string | null; onChange: (symbol: string) => Promise<boolean>; disabled?: boolean; saveError?: string; className?: string;
}) {
  const [open, setOpen] = useState(false);
  return <>
    <button type="button" aria-label="Instrument" aria-haspopup="dialog" aria-expanded={open} title={value ? `${value} · Select chart instrument` : "Select chart instrument"} disabled={disabled} className={`${className} flex items-center gap-2`} onClick={() => setOpen(true)}>
      {value ? <InstrumentLabel symbol={value} /> : "Instrument"}
      <svg aria-hidden="true" viewBox="0 0 20 20" fill="currentColor" className="size-3 shrink-0 text-zinc-500"><path d="m5 8 5 5 5-5Z" /></svg>
    </button>
    {open && <SingleInstrumentDialog instruments={instruments} value={value} pending={disabled} saveError={saveError} onChange={onChange} onCancel={() => setOpen(false)} onSuccess={() => setOpen(false)} />}
  </>;
}

function SingleInstrumentDialog({ instruments, value, pending, saveError, onChange, onCancel, onSuccess }: {
  instruments: string[]; value: string | null; pending: boolean; saveError: string; onChange: (symbol: string) => Promise<boolean>; onCancel: () => void; onSuccess: () => void;
}) {
  const name = useId();
  const [selected, setSelected] = useState(value ?? ""), [search, setSearch] = useState("");
  const [applying, setApplying] = useState(false), [error, setError] = useState("");
  const busy = useRef(false), alive = useRef(true), results = useRef<HTMLFieldSetElement>(null);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  const active = new Set(instruments), term = search.trim().toLowerCase();
  const symbols = [...new Set([...instruments, ...(value ? [value] : [])])].sort();
  const matches = symbols.filter((symbol) => symbol.toLowerCase().includes(term));
  const selectable = matches.filter((symbol) => active.has(symbol));
  const canApply = selectable.includes(selected), blocked = pending || applying;
  useEffect(() => {
    results.current?.querySelector("input:checked")?.closest("label")?.scrollIntoView({ block: "nearest" });
  }, [selected, term]);
  function filter(value: string) {
    setSearch(value);
    const nextTerm = value.trim().toLowerCase();
    setSelected(symbols.find((symbol) => active.has(symbol) && symbol.toLowerCase().includes(nextTerm)) ?? "");
  }
  function onSearchKey(event: KeyboardEvent<HTMLElement>) {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Enter") {
      event.preventDefault();
      if (!event.repeat) void apply();
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    if (blocked || !selectable.length) return;
    const index = selectable.indexOf(selected), down = event.key === "ArrowDown";
    const next = index < 0 ? (down ? 0 : selectable.length - 1) : Math.max(0, Math.min(selectable.length - 1, index + (down ? 1 : -1)));
    setSelected(selectable[next]!);
  }
  async function apply() {
    if (blocked || busy.current || !canApply) return;
    busy.current = true; setApplying(true); setError("");
    try {
      const saved = await onChange(selected);
      if (alive.current) { if (saved) onSuccess(); else setError("Unable to change instrument. Please try again."); }
    } catch (cause) { if (alive.current) setError(cause instanceof Error ? cause.message : "Unable to change instrument."); }
    finally { busy.current = false; if (alive.current) setApplying(false); }
  }
  return <ConfigDialog title="Select chart instrument" onCancel={onCancel} onApply={() => { void apply(); }} applyDisabled={blocked || !canApply}>
    <label className="block shrink-0 space-y-2 text-sm">Search instruments<input autoFocus type="search" value={search} disabled={blocked} onChange={(event) => filter(event.target.value)} onKeyDown={onSearchKey} aria-controls={`${name}-results`} aria-describedby={`${name}-keys`} className={configInput} placeholder="Filter instruments" autoComplete="off" /></label>
    <p id={`${name}-keys`} className="text-xs text-zinc-500">↑ / ↓ to choose · Enter to apply · Esc to cancel</p>
    <p role="status" className="sr-only">{canApply ? `${selected} selected` : selectable.length ? "Choose an active instrument" : "No matching active instruments"}</p>
    {(saveError || error) && <p role="alert" className="text-sm text-red-300">{saveError || error}</p>}
    {!instruments.length && <p role="status" className="text-sm text-amber-200">No active perpetual instruments are available.</p>}
    <fieldset ref={results} id={`${name}-results`} disabled={blocked} aria-label="Chart instruments" onKeyDown={(event) => { if (event.key === "Enter") onSearchKey(event); }} className="min-h-0 space-y-2 overflow-y-auto pb-5 disabled:opacity-50" style={{ maxHeight: "24rem" }}>
      {matches.map((symbol) => <label key={symbol} className={instrumentRow}>
        <input type="radio" name={name} aria-label={symbol} checked={selected === symbol} disabled={!active.has(symbol)} onChange={() => setSelected(symbol)} className={radio} />
        <InstrumentLabel symbol={symbol} />{!active.has(symbol) && <span className="ml-auto text-xs text-amber-200">Unavailable</span>}
      </label>)}
      {!matches.length && instruments.length > 0 && <p className="rounded-xl border border-zinc-800 p-6 text-center text-sm text-zinc-500">No instruments match this filter.</p>}
    </fieldset>
  </ConfigDialog>;
}

import { useState } from "react";
import type { SubagentRow } from "../db/subagent-schema.js";
import { candleIntervals } from "../agents/candle-intervals.js";
import { workflowDraft } from "../agents/subagent-templates.js";
import { workerWorkflow, workflowReadiness, type WorkerWorkflow } from "../agents/worker-workflow.js";

const input = "mt-1 block w-full rounded-lg border border-zinc-700 bg-zinc-950 p-2 text-sm text-white";
const button = "cursor-pointer rounded-lg border border-zinc-700 px-3 py-2 text-sm hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-50";
export function WorkerWorkflowConfig({ value, workers, disabled, onSave }: { value?: WorkerWorkflow | null; workers: SubagentRow[]; disabled?: boolean; onSave: (workflow: WorkerWorkflow | null) => Promise<void> }) {
  const [draft, setDraft] = useState<WorkerWorkflow | null>(value ?? null), [busy, setBusy] = useState(false), [error, setError] = useState("");
  const [coins, setCoins] = useState(value && "instruments" in value ? value.instruments.join(", ") : "");
  const patch = (change: Record<string, unknown>) => setDraft((current) => current ? { ...current, ...change } as WorkerWorkflow : null);
  async function save() {
    setBusy(true); setError("");
    try {
      const instruments = coins.split(",").map((item) => item.trim()).filter(Boolean);
      const candidate = draft && "instruments" in draft ? { ...draft, instruments, ...(draft.purpose === "trading" ? { baseSizeByInstrument: Object.fromEntries(instruments.map((coin) => [coin, draft.baseSizeByInstrument[coin] ?? ""])) } : {}) } : draft;
      const normalized = candidate ? workerWorkflow(candidate) : null;
      await onSave(normalized); setDraft(normalized);
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to save workflow; your draft is preserved"); }
    finally { setBusy(false); }
  }
  const instruments = coins.split(",").map((coin) => coin.trim()).filter(Boolean);
  const readiness = draft ? workflowReadiness(draft, workers) : [];
  return <section className="space-y-4 rounded-xl border border-zinc-800 p-4" aria-label="Workflow configuration">
    <h3 className="font-medium">Workflow & enforced entry settings</h3><p className="text-xs text-zinc-400">Ordinary editable worker configuration, not protected worker kinds. Learnings cannot change these settings. Schedules do not establish analyst/trader ordering guarantees.</p>
    <fieldset className="space-y-3" disabled={disabled || busy}>
      <label>Purpose<select className={`${input} cursor-pointer`} value={draft?.purpose ?? ""} onChange={(event) => { if (draft && !window.confirm("Replace the unsaved workflow fields with new-purpose defaults?")) return; const purpose = event.target.value as WorkerWorkflow["purpose"] | ""; setDraft(purpose ? workflowDraft(purpose) : null); setCoins(""); }}><option value="">No workflow (automatic entries fail closed)</option>{["analysis", "trading", "review"].map((purpose) => <option key={purpose}>{purpose}</option>)}</select></label>
      {draft && "instruments" in draft && <label>Exact main-DEX perpetual instruments (comma separated, max 10)<input aria-label="Workflow instruments" className={input} maxLength={330} value={coins} onChange={(event) => setCoins(event.target.value)} /></label>}
      {draft?.purpose === "analysis" && <>
        <label>Analytical interval<select className={`${input} cursor-pointer`} value={draft.interval} onChange={(event) => patch({ interval: event.target.value })}>{Object.keys(candleIntervals).map((interval) => <option key={interval}>{interval}</option>)}</select></label><label>Research topic<input className={input} maxLength={64} value={draft.topic} onChange={(event) => patch({ topic: event.target.value })} /></label>
        <label>Maximum research age (seconds from evidence boundary)<input type="number" min={1} max={604800} className={input} value={draft.maxResearchAgeSeconds} onChange={(event) => patch({ maxResearchAgeSeconds: Number(event.target.value) })} /></label><label>Analysis strategy<textarea className={input} rows={4} maxLength={8000} value={draft.strategy} onChange={(event) => patch({ strategy: event.target.value })} /></label>
      </>}
      {draft?.purpose === "trading" && <>
        <label className="flex cursor-pointer items-center gap-2"><input type="checkbox" className="cursor-pointer" checked={draft.automaticEntriesEnabled} onChange={(event) => patch({ automaticEntriesEnabled: event.target.checked })} />Explicitly allow automatic NEW exposure</label>
        <p className="text-sm text-amber-200">Off by default. The starter host policy is long-only and non-pyramiding; edited instructions cannot bypass it. Template creation is always paused with automatic entries OFF; enable only after reviewing the saved copy. Scheduled repair cannot guarantee zero unprotected time after partial fills or venue failures. Host-verified user-directed trades may omit recent research, but not limits/current data/full-size TP & SL. Disabling this worker does not flatten positions or cancel orders.</p>
        {instruments.map((coin) => <label key={coin}>Base quantity · {coin}<input aria-label={`Base quantity ${coin}`} className={input} inputMode="decimal" value={draft.baseSizeByInstrument[coin] ?? ""} onChange={(event) => patch({ baseSizeByInstrument: { ...draft.baseSizeByInstrument, [coin]: event.target.value } })} /></label>)}
        <label>Maximum entry notional (exact USDC string)<input className={input} inputMode="decimal" value={draft.maxEntryNotional} onChange={(event) => patch({ maxEntryNotional: event.target.value })} /></label>
        <div className="grid gap-3 sm:grid-cols-2"><label>Take-profit distance (integer bps)<input className={input} type="number" min={1} max={9999} value={draft.takeProfitBps} onChange={(event) => patch({ takeProfitBps: Number(event.target.value) })} /></label><label>Stop-loss distance (integer bps)<input className={input} type="number" min={1} max={9999} value={draft.stopLossBps} onChange={(event) => patch({ stopLossBps: Number(event.target.value) })} /></label></div>
        <h4>Mandatory analyst streams (all required)</h4>{draft.requiredResearch.map((requirement, index) => {
          const change = (value: Record<string, unknown>) => patch({ requiredResearch: draft.requiredResearch.map((item, i) => i === index ? { ...item, ...value } : item) });
          return <div className="space-y-2 rounded-lg border border-zinc-700 p-3" key={index}>
            <label>Same-Agent producer<select className={`${input} cursor-pointer`} value={requirement.subagentId} onChange={(event) => { const producer = workers.find((worker) => worker.id === event.target.value), workflow = producer?.workflow; change({ subagentId: event.target.value, ...(workflow?.purpose === "analysis" ? { topic: workflow.topic, interval: workflow.interval, maxAgeSeconds: workflow.maxResearchAgeSeconds } : {}) }); }}><option value="">Select configured analyst</option>{workers.filter((worker) => worker.kind === "worker" && worker.workflow?.purpose === "analysis").map((worker) => <option key={worker.id} value={worker.id}>{worker.name}{worker.archived || !worker.enabled ? " (disabled/archived)" : ""}</option>)}</select></label>
            <label>Topic<input className={input} value={requirement.topic} onChange={(event) => change({ topic: event.target.value })} /></label><label>Interval<select className={`${input} cursor-pointer`} value={requirement.interval} onChange={(event) => change({ interval: event.target.value })}>{Object.keys(candleIntervals).map((interval) => <option key={interval}>{interval}</option>)}</select></label><label>Maximum acceptable age (seconds)<input className={input} type="number" min={1} max={604800} value={requirement.maxAgeSeconds} onChange={(event) => change({ maxAgeSeconds: Number(event.target.value) })} /></label>
            <button type="button" className={button} onClick={() => patch({ requiredResearch: draft.requiredResearch.filter((_, i) => i !== index) })}>Remove stream</button>
          </div>;
        })}<button type="button" className={button} disabled={draft.requiredResearch.length >= 20} onClick={() => patch({ requiredResearch: [...draft.requiredResearch, { subagentId: "", topic: "trend_setup", interval: "1h", maxAgeSeconds: 3600 }] })}>Add required analyst stream</button>
        <label>Entry strategy<textarea className={input} maxLength={8000} rows={4} value={draft.entryStrategy} onChange={(event) => patch({ entryStrategy: event.target.value })} /></label><label>Exit strategy<textarea className={input} maxLength={8000} rows={4} value={draft.exitStrategy} onChange={(event) => patch({ exitStrategy: event.target.value })} /></label>
      </>}
      {draft?.purpose === "review" && <>
        <label>Lookback hours<input className={input} type="number" min={1} max={2160} value={draft.lookbackHours} onChange={(event) => patch({ lookbackHours: Number(event.target.value) })} /></label><p className="text-sm">Agent-wide learnings are writable. Also permit these exact worker targets:</p>
        {workers.filter((worker) => worker.kind === "worker").map((worker) => <label className="flex cursor-pointer items-center gap-2 text-sm" key={worker.id}><input className="cursor-pointer" type="checkbox" checked={draft.learningTargetSubagentIds.includes(worker.id)} onChange={(event) => patch({ learningTargetSubagentIds: event.target.checked ? [...draft.learningTargetSubagentIds, worker.id] : draft.learningTargetSubagentIds.filter((id) => id !== worker.id) })} />{worker.name}{worker.archived ? " (archived history)" : ""}</label>)}
      </>}
      {readiness.map((warning) => <p key={warning} role="status" className="text-sm text-amber-200">{warning}</p>)}
      <button type="button" className={button} onClick={() => { void save(); }}>{busy ? "Saving…" : "Save workflow settings"}</button>
    </fieldset>{error && <p role="alert" className="text-sm text-red-300">{error}</p>}
  </section>;
}

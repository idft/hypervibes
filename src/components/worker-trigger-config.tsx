import { useRef, useState } from "react";
import { normalizeDelegationTools } from "../agents/subagent-input.js";
import { defaultWorkerTriggers, type WorkerTriggers } from "../agents/worker-triggers.js";
import { ConfigDialog, configButton } from "./config-dialog.js";

type TriggerWorker = { id: string; name: string; archived?: boolean; tools?: string[] };

export function WorkerTriggerConfig({ value = defaultWorkerTriggers, workers, onChange }: { value?: WorkerTriggers; workers: TriggerWorker[]; onChange: (value: WorkerTriggers) => Promise<void> }) {
  const [open, setOpen] = useState(false);
  return <section className="space-y-2">
    <header className="flex items-center justify-between gap-4"><h3 className="font-medium text-white">Event triggers</h3><button type="button" aria-haspopup="dialog" onClick={() => setOpen(true)} className={configButton}>Edit triggers</button></header>
    <p className="text-sm text-zinc-400">{value.inboxEnabled ? "Event-driven inbox enabled" : "Event-driven inbox disabled"}</p>
    {value.inboxEnabled && <p className="text-sm text-zinc-400">{[value.researchSources.length ? `${value.researchSources.length} research sources` : "", value.fills ? "Order fills" : "", value.orderChanges ? "Order status changes" : ""].filter(Boolean).join(" · ") || "No automatic events selected"} · {value.workerDelegates.length} approved worker callers</p>}
    {open && <TriggerDialog value={value} workers={workers} onCancel={() => setOpen(false)} onApply={async (next) => { await onChange(next); setOpen(false); }} />}
  </section>;
}

function TriggerDialog({ value, workers, onCancel, onApply }: { value: WorkerTriggers; workers: TriggerWorker[]; onCancel: () => void; onApply: (value: WorkerTriggers) => Promise<void> }) {
  const [draft, setDraft] = useState(value);
  const [saving, setSaving] = useState(false), [error, setError] = useState("");
  const inFlight = useRef(false);
  async function apply() {
    if (inFlight.current) return;
    inFlight.current = true; setSaving(true); setError("");
    try { await onApply(draft); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to save triggers"); }
    finally { inFlight.current = false; setSaving(false); }
  }
  const sources = (key: "researchSources" | "workerDelegates", title: string) => <fieldset className="space-y-2"><legend className="mb-2 text-sm font-medium text-white">{title}</legend>{workers.length ? workers.map((worker) => <label key={worker.id} className="flex cursor-pointer items-center gap-3 text-sm"><input type="checkbox" checked={draft[key].includes(worker.id)} disabled={worker.archived && !draft[key].includes(worker.id)} onChange={(event) => setDraft((current) => ({ ...current, [key]: event.target.checked ? [...current[key], worker.id] : current[key].filter((id) => id !== worker.id) }))} className="cursor-pointer" />{worker.name}{worker.archived ? " (archived)" : ""}</label>) : <p className="text-xs text-zinc-500">Create another worker first.</p>}</fieldset>;
  return <ConfigDialog title="Event triggers" onCancel={() => { if (!inFlight.current) onCancel(); }} onApply={() => { void apply(); }} applyDisabled={saving}>
    <fieldset disabled={saving} className="space-y-5 disabled:opacity-60">
      <label className="flex cursor-pointer items-center gap-3 text-sm"><input type="checkbox" checked={draft.inboxEnabled} onChange={(event) => setDraft(event.target.checked ? { ...draft, inboxEnabled: true } : { ...defaultWorkerTriggers })} className="cursor-pointer" />Enable event triggers</label>
      {draft.inboxEnabled && <>
        {sources("researchSources", "Research published")}
        {sources("workerDelegates", "Allow research delegation from")}
        {workers.filter((worker) => draft.workerDelegates.includes(worker.id) && worker.tools !== undefined && !normalizeDelegationTools(worker.tools).includes("delegate_subagent")).map((worker) => <p key={worker.id} role="status" className="text-xs text-amber-200">{worker.name} cannot delegate: enable delegate_subagent in its Tools.</p>)}
        <label className="flex cursor-pointer items-center gap-3 text-sm"><input type="checkbox" checked={draft.fills} onChange={(event) => setDraft((current) => ({ ...current, fills: event.target.checked }))} className="cursor-pointer" />Order fills — partial or complete</label>
        <label className="flex cursor-pointer items-center gap-3 text-sm"><input type="checkbox" checked={draft.orderChanges} onChange={(event) => setDraft((current) => ({ ...current, orderChanges: event.target.checked }))} className="cursor-pointer" />Order status changes — canceled, rejected, expired</label>
      </>}
    </fieldset>
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
  </ConfigDialog>;
}

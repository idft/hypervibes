import { useState } from "react";
import { chartIntervals } from "../hyperliquid/chart-contracts.js";
import { drawing, type ChartDrawing } from "../charts/contracts.js";
import { ConfigDialog, configInput } from "./config-dialog.js";

export function ChartDrawingEditor({ value, pending, saveError, onCancel, onApply }: { value: ChartDrawing; pending: boolean; saveError: string; onCancel: () => void; onApply: (value: ChartDrawing) => Promise<boolean> }) {
  const [draft, setDraft] = useState(value), [error, setError] = useState("");
  async function apply() {
    try { const parsed = drawing(draft); if (await onApply(parsed)) onCancel(); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Invalid drawing settings"); }
  }
  return <ConfigDialog title="Drawing settings" onCancel={onCancel} onApply={() => { void apply(); }} applyDisabled={pending || value.locked}>
    <fieldset disabled={pending || value.locked} className="space-y-4 disabled:opacity-50">
      <label className="block space-y-1 text-sm">Name<input autoFocus className={configInput} value={draft.name} maxLength={100} onChange={(e) => setDraft({ ...draft, name: e.target.value })} /></label>
      <label className="block space-y-1 text-sm">Text<textarea className={configInput} value={draft.text} rows={3} maxLength={1000} onChange={(e) => setDraft({ ...draft, text: e.target.value })} /></label>
      <label className="block space-y-1 text-sm">Group<input className={configInput} value={draft.group} maxLength={80} onChange={(e) => setDraft({ ...draft, group: e.target.value })} /></label>
      <div className="flex flex-wrap items-center gap-4">
        <label className="flex items-center gap-2 text-sm">Color<input type="color" className="cursor-pointer" value={draft.color} onChange={(e) => setDraft({ ...draft, color: e.target.value })} /></label>
        <label className="flex items-center gap-2 text-sm">Width<select className="cursor-pointer rounded bg-zinc-900 p-2" value={draft.width} onChange={(e) => setDraft({ ...draft, width: Number(e.target.value) })}>{[1,2,3,4,5].map((n) => <option key={n}>{n}</option>)}</select></label>
        <label className="cursor-pointer text-sm"><input type="checkbox" className="mr-2 cursor-pointer" checked={draft.dashed} onChange={(e) => setDraft({ ...draft, dashed: e.target.checked })} />Dashed</label>
      </div>
      {draft.kind === "rectangle" && <label className="block text-sm">Fill opacity<input type="range" className="ml-3 cursor-pointer" min="0" max="0.6" step="0.05" value={draft.fillOpacity} onChange={(e) => setDraft({ ...draft, fillOpacity: Number(e.target.value) })} /></label>}
      <fieldset className="space-y-2"><legend className="mb-2 text-sm">Coordinates · UTC seconds / price</legend>{draft.points.map((point, i) => <div key={i} className="grid grid-cols-2 gap-3">
        <label className="text-xs text-zinc-400">Anchor {i + 1} time<input type="number" step="any" className={configInput} value={point.time} onChange={(e) => setDraft({ ...draft, points: draft.points.map((p, n) => n === i ? { ...p, time: Number(e.target.value) } : p) })} /></label>
        <label className="text-xs text-zinc-400">Anchor {i + 1} price<input type="number" step="any" className={configInput} value={point.price} onChange={(e) => setDraft({ ...draft, points: draft.points.map((p, n) => n === i ? { ...p, price: Number(e.target.value) } : p) })} /></label>
      </div>)}</fieldset>
      <fieldset><legend className="mb-2 text-sm">Visible timeframes · none selected means all</legend><div className="flex flex-wrap gap-3">{chartIntervals.map((interval) => <label key={interval} className="cursor-pointer text-xs"><input type="checkbox" className="mr-1 cursor-pointer" checked={draft.intervals.includes(interval)} onChange={(e) => setDraft({ ...draft, intervals: e.target.checked ? [...draft.intervals, interval] : draft.intervals.filter((i) => i !== interval) })} />{interval}</label>)}</div></fieldset>
    </fieldset>
    {(error || saveError) && <p role="alert" className="mt-3 text-sm text-red-300">{error || saveError}</p>}
  </ConfigDialog>;
}

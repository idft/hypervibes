import { useState } from "react";
import { useQueries, useQuery } from "@tanstack/react-query";
import type { ChartIndicatorInstance } from "../indicators/contracts.js";
import { indicatorListQueryOptions, indicatorQueryOptions } from "../web/indicator-queries.js";
import { ConfigDialog, configButton, configInput } from "./config-dialog.js";
import { IndicatorInputFields } from "./indicator-input-fields.js";
const iconButton = "inline-flex h-8 w-8 cursor-pointer items-center justify-center rounded-lg text-zinc-400 hover:bg-zinc-800 hover:text-white disabled:cursor-not-allowed disabled:opacity-40";
function IndicatorActionIcon({ name }: { name: "eye" | "eye-off" | "gear" | "trash" | "up" }) {
  const paths = {
    eye: "M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0",
    "eye-off": "m3 3 18 18 M10.6 5.1A12 12 0 0 1 12 5c6.5 0 10 7 10 7a17 17 0 0 1-3 4 M6.5 6.5A22 22 0 0 0 2 12s3.5 7 10 7a12 12 0 0 0 5.5-1.5 M10 10a3 3 0 0 0 4 4",
    gear: "m9 3-.5 3-2 1-3-.5-2 3 2 2v2l-2 2 2 3 3-.5 2 1 .5 3h4l.5-3 2-1 3 .5 2-3-2-2v-2l2-2-2-3-3 .5-2-1L13 3Z M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0",
    trash: "M3 6h18 M9 6V3h6v3 M5 6l1 15h12l1-15 M10 10v7 M14 10v7",
    up: "m6 12 6-6 6 6 M12 6v14",
  };
  return <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" className="h-4 w-4"><path d={paths[name]} /></svg>;
}

export function ChartIndicatorControls({ agentId, instances, disabled, onChange }: { agentId: string; instances: ChartIndicatorInstance[]; disabled: boolean; onChange: (instances: ChartIndicatorInstance[]) => Promise<boolean> }) {
  const [open, setOpen] = useState(false), [editing, setEditing] = useState<ChartIndicatorInstance | null>(null);
  const [adding, setAdding] = useState(false);
  const [query, setQuery] = useState(""), [offset, setOffset] = useState(0);
  const catalog = useQuery({ ...indicatorListQueryOptions(agentId, { query, offset, limit: 20 }), enabled: open });
  const indicatorIds = [...new Set(instances.map((instance) => instance.indicatorId))];
  const details = useQueries({ queries: indicatorIds.map((indicatorId) => ({ ...indicatorQueryOptions(agentId, indicatorId), enabled: open })) });
  return <>
    <button type="button" className={configButton} aria-label="Indicators" disabled={disabled} onClick={() => setOpen(true)}>Indicators ({instances.length})</button>
    {open && !editing && <ConfigDialog title="Chart indicators" onCancel={() => setOpen(false)} onApply={() => setOpen(false)} applyLabel="Done">
      {instances.length > 0 && <div className="overflow-x-auto rounded-xl border border-zinc-800"><table aria-label="Indicators on chart" className="w-full text-left text-sm">
        <thead className="border-b border-zinc-800 text-zinc-400"><tr><th scope="col" className="px-4 py-3 font-medium">Indicator</th><th scope="col" className="px-4 py-3 text-right font-medium">Actions</th></tr></thead>
        <tbody className="divide-y divide-zinc-800">{instances.map((instance, index) => {
          const detail = details[indicatorIds.indexOf(instance.indicatorId)];
          const name = detail?.data?.descriptor.name ?? (detail?.isError ? "Unavailable indicator" : "Loading indicator…");
          const visibility = `${instance.hidden ? "Show" : "Hide"} ${name}`;
          return <tr key={instance.id}>
            <td className={`px-4 py-3 ${instance.hidden ? "text-zinc-500" : "text-zinc-200"}`}>{name}</td>
            <td className="px-4 py-2"><div className="flex justify-end gap-1">
              <button type="button" className={iconButton} aria-label={visibility} title={visibility} disabled={disabled} onClick={() => { void onChange(instances.map((v) => v.id === instance.id ? { ...v, hidden: !v.hidden } : v)); }}><IndicatorActionIcon name={instance.hidden ? "eye-off" : "eye"} /></button>
              <button type="button" className={iconButton} aria-label={`Configure ${name}`} title={`Configure ${name}`} disabled={disabled} onClick={() => { setAdding(false); setEditing(instance); }}><IndicatorActionIcon name="gear" /></button>
              <button type="button" className={iconButton} aria-label={`Move ${name} up`} title={`Move ${name} up`} disabled={disabled || index === 0} onClick={() => { const next = [...instances]; [next[index - 1], next[index]] = [next[index]!, next[index - 1]!]; void onChange(next); }}><IndicatorActionIcon name="up" /></button>
              <button type="button" className={`${iconButton} hover:text-red-300`} aria-label={`Remove ${name}`} title={`Remove ${name}`} disabled={disabled} onClick={() => { void onChange(instances.filter((v) => v.id !== instance.id)); }}><IndicatorActionIcon name="trash" /></button>
            </div></td>
          </tr>;
        })}</tbody>
      </table></div>}
      <label className="my-3 block text-sm">Search<input className={configInput} value={query} onChange={(e) => { setQuery(e.target.value); setOffset(0); }} /></label>
      {catalog.error && <p role="alert">{catalog.error.message}</p>}
      <ul className="space-y-2">{catalog.data?.indicators.map((entry) => <li key={entry.id} className="flex items-center justify-between gap-2 text-sm"><span>{entry.name} · {entry.builtIn ? "Built-in" : `Custom · v${entry.version}`}</span><button type="button" className={configButton} disabled={disabled || instances.length >= 10} onClick={() => { setAdding(true); setEditing({ id: crypto.randomUUID(), indicatorId: entry.id, configuredVersion: entry.version, inputs: {}, pane: "auto", hidden: false, styles: {} }); }}>Add</button></li>)}</ul>
      <div className="mt-3 flex gap-2"><button type="button" className={configButton} disabled={!offset} onClick={() => setOffset(Math.max(0, offset - 20))}>Previous</button><button type="button" className={configButton} disabled={catalog.data?.nextOffset == null} onClick={() => setOffset(catalog.data!.nextOffset!)}>Next</button></div>
    </ConfigDialog>}
    {editing && <IndicatorInstanceDialog key={editing.id} agentId={agentId} initial={editing} adding={adding} disabled={disabled || (adding && instances.length >= 10)} onCancel={() => setEditing(null)} onApply={async (value) => { if (await onChange(adding ? [...instances, value] : instances.map((v) => v.id === value.id ? value : v))) { setEditing(null); if (adding) setOpen(false); } }} />}
  </>;
}
function IndicatorInstanceDialog({ agentId, initial, adding, disabled, onCancel, onApply }: { agentId: string; initial: ChartIndicatorInstance; adding: boolean; disabled: boolean; onCancel: () => void; onApply: (value: ChartIndicatorInstance) => Promise<void> }) {
  const [draft, setDraft] = useState(initial), [styles, setStyles] = useState(JSON.stringify(initial.styles)), [error, setError] = useState("");
  const detail = useQuery(indicatorQueryOptions(agentId, initial.indicatorId));
  return <ConfigDialog title={adding ? "Add indicator" : "Configure indicator"} applyLabel={adding ? "Add to chart" : "Apply"} onCancel={onCancel} applyDisabled={disabled || !detail.data} onApply={() => { try { const parsed: unknown = JSON.parse(styles); if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("Styles must be an object keyed by output ID"); void onApply({ ...draft, configuredVersion: detail.data!.descriptor.version, styles: parsed as ChartIndicatorInstance["styles"] }).catch((e: unknown) => setError(e instanceof Error ? e.message : "Unable to save settings")); } catch (e) { setError(e instanceof Error ? e.message : "Invalid styles"); } }}>
    {detail.data && <IndicatorInputFields schema={detail.data.descriptor.inputs} value={draft.inputs} onChange={(inputs) => setDraft({ ...draft, inputs })} disabled={disabled} />}
    <details className="rounded-xl border border-zinc-800 p-3">
      <summary className="cursor-pointer text-sm text-zinc-400 hover:text-white">Advanced options</summary>
    <label className="my-3 block text-sm">Pane<select className={`${configInput} cursor-pointer`} value={draft.pane} onChange={(e) => setDraft({ ...draft, pane: e.target.value as ChartIndicatorInstance["pane"] })}><option value="auto">Default</option><option value="price">Price pane</option><option value="separate">Separate pane</option></select></label>
    <label className="block text-sm">Output styles (JSON keyed by exact output ID)<textarea rows={4} className={`${configInput} font-mono`} value={styles} onChange={(e) => setStyles(e.target.value)} /></label><p className="text-xs text-zinc-500">Example: {`{"EMA":{"color":"#ff9900","width":2,"hidden":false}}`}. display.none cannot be overridden.</p>
    </details>
    {(error || detail.error) && <p role="alert" className="text-red-300">{error || detail.error?.message}</p>}
  </ConfigDialog>;
}

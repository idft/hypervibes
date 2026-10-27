import { randomUuid } from "../web/random-uuid.js";
import { useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getIndicator, previewIndicator, previewSavedIndicator, saveIndicator } from "../web/indicators.js";
import { indicatorKeys, indicatorQueryOptions, indicatorVersionsQueryOptions } from "../web/indicator-queries.js";
import type { IndicatorDescriptor, IndicatorInputs } from "../indicators/contracts.js";
import { chartIntervals } from "../hyperliquid/chart-contracts.js";
import { configButton, configInput } from "./config-dialog.js";
import { IndicatorInputFields } from "./indicator-input-fields.js";
import { IndicatorPreviewChart } from "./indicator-preview-chart.js";
const initialSource = '//@version=6\nindicator("My indicator", overlay=true)\nperiod = input.int(20, "Period", minval=2, maxval=500)\nplot(ta.ema(close, period), "EMA", color=color.orange)\n';
export function IndicatorDetailPage({ agentId, indicatorId, copy }: { agentId: string; indicatorId: string; copy?: string }) {
  const [viewVersion, setViewVersion] = useState<number | undefined>(undefined), [offset, setOffset] = useState(0);
  const sourceId = indicatorId === "new" ? copy : indicatorId;
  const detail = useQuery({ ...indicatorQueryOptions(agentId, sourceId ?? "builtin:ema", { version: viewVersion }), enabled: Boolean(sourceId) });
  const source = useQuery({ queryKey: [...indicatorKeys.all(agentId), "editor-source", sourceId, viewVersion, detail.data?.descriptor.sourceHash], enabled: Boolean(sourceId && detail.data), queryFn: async ({ signal }) => {
    let text = "", offset = 0;
    do { const page = await getIndicator({ data: { agentId, indicatorId: sourceId!, version: detail.data!.descriptor.version, includeSource: true, sourceOffset: offset, sourceLimit: 16000 }, signal }); text += page.source!.text; if (page.source!.nextOffset === null) break; offset = page.source!.nextOffset; } while (offset < 65536);
    return text;
  } });
  const versions = useQuery({ ...indicatorVersionsQueryOptions(agentId, sourceId ?? "builtin:ema", offset), enabled: Boolean(sourceId) });
  const current = useQuery({ ...indicatorQueryOptions(agentId, sourceId ?? "builtin:ema"), enabled: Boolean(sourceId) });
  if (sourceId && (!detail.data || source.data === undefined || !current.data)) return <p role={detail.error || source.error ? "alert" : "status"} className="p-4">{detail.error?.message ?? source.error?.message ?? "Loading indicator…"}</p>;
  return <section className="w-full space-y-5 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
    <IndicatorEditor key={`${sourceId ?? "new"}:${detail.data?.descriptor.version ?? 0}:${indicatorId}`} agentId={agentId} indicatorId={indicatorId} descriptor={detail.data?.descriptor} currentVersion={current.data?.descriptor.version} initial={source.data ?? initialSource} />
    {sourceId && <section className="space-y-3"><h3 className="text-lg">Source history (readable and executable; no activation)</h3><button className={configButton} onClick={() => setViewVersion(undefined)}>View current</button><ul className="space-y-2">{versions.data?.versions.map((entry) => <li key={entry.version}><button className={configButton} onClick={() => setViewVersion(entry.version)}>View / preview v{entry.version}</button><span className="ml-2 font-mono text-xs">{entry.sourceHash.slice(0, 12)}</span></li>)}</ul><button className={configButton} disabled={!offset} onClick={() => setOffset(Math.max(0, offset - 20))}>Previous versions</button><button className={configButton} disabled={versions.data?.nextOffset == null} onClick={() => setOffset(versions.data!.nextOffset!)}>More versions</button>{versions.error && <p role="alert">{versions.error.message}</p>}</section>}
  </section>;
}
function IndicatorEditor({ agentId, indicatorId, descriptor, currentVersion, initial }: { agentId: string; indicatorId: string; descriptor?: IndicatorDescriptor; currentVersion?: number; initial: string }) {
  const creating = indicatorId === "new", readOnly = !creating && descriptor?.builtIn === true;
  const [name, setName] = useState(creating && descriptor ? `${descriptor.name} copy` : descriptor?.name ?? "My indicator"), [description, setDescription] = useState(descriptor?.description ?? ""), [interpretation, setInterpretation] = useState(descriptor?.interpretation ?? "");
  const [source, setSource] = useState(initial), [inputs, setInputs] = useState<IndicatorInputs>({});
  const [coin, setCoin] = useState("BTC"), [interval, setInterval] = useState("1h");
  const [end, setEnd] = useState(() => new Date(Math.floor(Date.now() / 3600000) * 3600000).toISOString());
  const [start, setStart] = useState(() => new Date(Math.floor(Date.now() / 3600000) * 3600000 - 300 * 3600000).toISOString());
  const client = useQueryClient(), navigate = useNavigate();
  const preview = useMutation({ mutationFn: () => !creating && descriptor && source === initial ? previewSavedIndicator({ data: { agentId, request: { indicatorId, version: descriptor.version, coin, interval, start: Date.parse(start), end: Date.parse(end), inputs, count: 20 } } }) : previewIndicator({ data: { agentId, request: { source, coin, interval, start: Date.parse(start), end: Date.parse(end), inputs, count: 20 } } }) });
  const [operation, setOperation] = useState<{ body: string; key: string } | undefined>();
  const save = useMutation({ mutationFn: async () => {
    const request = { ...(creating ? {} : { indicatorId, expectedVersion: currentVersion }), name, description, interpretation, source, validation: { coin, interval, start: Date.parse(start), end: Date.parse(end) } };
    const body = JSON.stringify(request), key = operation?.body === body ? operation.key : randomUuid(); setOperation({ body, key });
    return saveIndicator({ data: { agentId, operationKey: key, request } });
  }, onSuccess: async (result) => { await client.invalidateQueries({ queryKey: indicatorKeys.all(agentId) }); await navigate({ to: "/agents/$agentId/indicators/$indicatorId", params: { agentId, indicatorId: result.indicatorId }, search: {} }); } });
  const schema = preview.data?.result.inputs ?? descriptor?.inputs ?? [];
  return <div className="space-y-4"><header className="flex flex-wrap items-center justify-between gap-3"><nav aria-label="Indicator breadcrumb" className="flex min-w-0 flex-1 flex-wrap items-center gap-2"><Link className="cursor-pointer text-sm text-zinc-400 hover:text-white" to="/agents/$agentId/indicators" params={{ agentId }} search={{ query: "", kind: "all", offset: 0 }}><span aria-hidden="true">← </span>Indicators</Link><span aria-hidden="true" className="text-zinc-600">/</span><h2 aria-current="page" className="break-words text-lg font-medium text-white">{creating ? "New indicator" : descriptor?.name}</h2></nav>{readOnly && <Link className={configButton} to="/agents/$agentId/indicators/$indicatorId" params={{ agentId, indicatorId: "new" }} search={{ copy: indicatorId }}>Copy to custom</Link>}</header>
    <hr className="border-zinc-800" />
    <fieldset disabled={readOnly || save.isPending} className="space-y-3"><label className="block text-sm">Name<input className={configInput} maxLength={100} value={name} onChange={(e) => setName(e.target.value)} /></label><label className="block text-sm">Description<textarea className={configInput} maxLength={2000} value={description} onChange={(e) => setDescription(e.target.value)} /></label><label className="block text-sm">Interpretation<textarea className={configInput} maxLength={8000} value={interpretation} onChange={(e) => setInterpretation(e.target.value)} /></label></fieldset>
    <label className="block text-sm">PineScript source<textarea readOnly={readOnly} disabled={save.isPending} rows={18} maxLength={65536} spellCheck={false} className={`${configInput} font-mono`} value={source} onChange={(e) => setSource(e.target.value)} /></label>
    <p className="text-sm text-zinc-400">{readOnly ? "Built-in source is read-only." : "Save immediately makes a new source version current; there is no publish/activation step. Reusing historical source saves it as a new current version."} Defaults belong in input.* source. Preview/chart overrides below do not change defaults or versions.</p>
    <h3 className="text-lg">Preview / save validation window</h3><div className="grid gap-3 md:grid-cols-2"><label className="text-sm">Instrument<input className={configInput} value={coin} maxLength={64} onChange={(e) => setCoin(e.target.value)} /></label><label className="text-sm">Timeframe<select className={`${configInput} cursor-pointer`} value={interval} onChange={(e) => setInterval(e.target.value)}>{chartIntervals.map((v) => <option key={v}>{v}</option>)}</select></label><label className="text-sm">Seed start (inclusive UTC ISO)<input className={configInput} value={start} onChange={(e) => setStart(e.target.value)} /></label><label className="text-sm">End (exclusive UTC ISO)<input className={configInput} value={end} onChange={(e) => setEnd(e.target.value)} /></label></div>
    <IndicatorInputFields schema={schema} value={inputs} onChange={setInputs} disabled={preview.isPending} /><button className={configButton} onClick={() => setInputs({})}>Reset preview overrides</button>
    <div className="flex gap-3"><button className={configButton} disabled={preview.isPending || save.isPending} onClick={() => preview.mutate()}>{preview.isPending ? "Calculating…" : "Preview"}</button>{!readOnly && <button className={configButton} disabled={save.isPending || preview.isPending} onClick={() => save.mutate()}>{save.isPending ? "Validating defaults and saving…" : "Save immediately"}</button>}</div>
    {(preview.error || save.error) && <p role="alert" className="text-red-300">{preview.error?.message ?? save.error?.message} Source conflicts require rereading current source before saving.</p>}
    {save.data && <p role="status">Saved {save.data.indicatorId} v{save.data.version}</p>}
    {preview.data && <section className="space-y-3"><p role="status">{preview.isPending ? "Previous preview (stale while calculating)" : "Preview complete"} · {preview.data.result.dataset.count} closed inputs · v{preview.data.result.version} · {preview.data.result.markers.length} signals · {preview.data.result.drawings.length} live drawings</p><p className="text-xs text-zinc-500">PineTS semantics apply: last supplied closed bar can be realtime. No TradingView parity or profitability claim. All-null plots are allowed; inspect availability below.</p><IndicatorPreviewChart result={preview.data.result} candles={preview.data.candles} /><pre className="max-h-96 overflow-auto rounded-xl bg-zinc-950 p-3 text-xs">{JSON.stringify({ outputs: preview.data.result.outputs.map((o) => ({ id: o.id, finiteCount: o.finiteCount, firstFiniteTime: o.firstFiniteTime })), latest: preview.data.result.values.slice(-20), markers: preview.data.result.markers.slice(-20), drawings: preview.data.result.drawings.slice(-20), warnings: preview.data.result.warnings }, null, 2)}</pre></section>}
  </div>;
}

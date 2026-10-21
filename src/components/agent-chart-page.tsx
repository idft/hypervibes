import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { chartIntervals, isChartInterval } from "../hyperliquid/chart-contracts.js";
import { drawingKinds, type ChartDrawing, type DrawingKind } from "../charts/contracts.js";
import type { AgentChat } from "../agents/agent-service.js";
import type { ModelChoice } from "../providers/provider-service.js";
import { AgentChatPage } from "./agent-chat-page.js";
import { MarketChart, type MarketChartHandle } from "./market-chart.js";
import { useChartLayouts } from "./use-chart-layouts.js";
import { ChartDrawingEditor } from "./chart-drawing-editor.js";
import { ChartLayoutSettings, type LayoutAction } from "./chart-layout-settings.js";
import { ConfigDialog, configInput } from "./config-dialog.js";
import { SingleInstrumentPicker } from "./instrument-picker.js";
import { defaultChartOverlays, type ChartOverlayVisibility } from "../charts/trading-contracts.js";

const control = "cursor-pointer rounded-lg border border-zinc-700 text-xs text-zinc-300 hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-40";
const button = `${control} px-2.5 py-1.5`;
const iconButton = `${control} flex size-8 shrink-0 items-center justify-center aria-pressed:border-violet-700 aria-pressed:bg-violet-950/50 aria-pressed:text-violet-200`;
const toolLabels: Record<DrawingKind | "select", string> = { select: "Select", horizontal: "Horizontal line", "horizontal-ray": "Horizontal ray", vertical: "Vertical line", trend: "Trend line", ray: "Ray", extended: "Extended line", rectangle: "Rectangle zone", text: "Text", callout: "Callout", arrow: "Arrow" };
const toolIcons: Record<DrawingKind | "select", string> = { select: "↖", horizontal: "━", "horizontal-ray": "╶", vertical: "│", trend: "╱", ray: "↗", extended: "⤢", rectangle: "▭", text: "T", callout: "☷", arrow: "➜" };
const iconPaths = {
  magnet: "M5 3v10a7 7 0 0 0 14 0V3h-4v10a3 3 0 0 1-6 0V3ZM5 7h4m6 0h4",
  eye: "M2 12s3-7 10-7 10 7 10 7-3 7-10 7S2 12 2 12Zm7 0a3 3 0 1 0 6 0 3 3 0 0 0-6 0",
  hidden: "m3 3 18 18M10 5.2a12 12 0 0 1 2-.2c7 0 10 7 10 7a19 19 0 0 1-3 4M6 6.5A19 19 0 0 0 2 12s3 7 10 7a13 13 0 0 0 5-1",
  tools: "m4 20 4-1L20 7l-4-4L4 15Zm10-15 4 4M4 15l4 4M14 16l5 5 3-3-5-5",
  lock: "M6 10h12v11H6ZM8 10V6a4 4 0 0 1 8 0v4m-4 5v2",
  unlock: "M6 10h12v11H6ZM8 10V6a4 4 0 0 1 8 0m-4 9v2",
  account: "M3 6h18M3 12h18M3 18h18M7 3v6m10 0v6m-7 0v6",
  chat: "M3 4h18v16H3ZM15 4v16m3-10 2 2-2 2",
  chatHidden: "M3 4h18v16H3ZM15 4v16m-4-10-2 2 2 2",
} as const;
function ChartIcon({ name }: { name: keyof typeof iconPaths }) {
  return <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" className="size-4"><path d={iconPaths[name]} /></svg>;
}
const minChatWidth = 240, maxChatWidth = 560;
export function AgentChartPage({ id, instruments, initialChat, choices }: { id: string; instruments: string[]; initialChat: AgentChat; choices: ModelChoice[] }) {
  const { layouts, layout, pending, error, ready, choose, create, remove, change } = useChartLayouts(id, instruments);
  const [tool, setTool] = useState<DrawingKind | "select">("select"), [selected, setSelected] = useState("");
  const [chatOpen, setChatOpen] = useState(true), [chatWidth, setChatWidth] = useState(360);
  const [editing, setEditing] = useState(false), [keepDrawing, setKeepDrawing] = useState(false), [toolsOpen, setToolsOpen] = useState(false);
  const [nameDialog, setNameDialog] = useState<"new" | "duplicate" | "rename" | null>(null), [notesOpen, setNotesOpen] = useState(false);
  const [overlayDialog, setOverlayDialog] = useState(false), [overlays, setOverlays] = useState(defaultChartOverlays);
  const overlayPreferences = `hypervibes:chart-overlays:v1:${id}`;
  useEffect(() => {
    let value = defaultChartOverlays;
    try {
      const saved: unknown = JSON.parse(localStorage.getItem(overlayPreferences) ?? "null");
      if (saved && typeof saved === "object" && "trades" in saved && typeof saved.trades === "boolean" && "position" in saved && typeof saved.position === "boolean" && "orders" in saved && typeof saved.orders === "boolean") value = { trades: saved.trades, position: saved.position, orders: saved.orders };
    } catch { /* Optional local preference; never part of layout revisions. */ }
    setOverlays(value);
  }, [overlayPreferences]);
  const chart = useRef<MarketChartHandle>(null), workspace = useRef<HTMLElement>(null), canvas = useRef<HTMLDivElement>(null);
  const [chatLimit, setChatLimit] = useState(maxChatWidth);
  const [chartStatusHost, setChartStatusHost] = useState<HTMLDivElement | null>(null);
  const splitWidth = Math.min(chatWidth, chatLimit);
  const clampWidth = (width: number) => Math.max(minChatWidth, Math.min(chatLimit, width));
  useEffect(() => {
    if (!workspace.current || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(([entry]) => { if (entry) setChatLimit(Math.max(minChatWidth, Math.min(maxChatWidth, Math.floor(entry.contentRect.width - 292)))); });
    observer.observe(workspace.current);
    return () => observer.disconnect();
  }, []);
  const resize = useRef<{ x: number; width: number; pointerId: number } | null>(null);
  const preferences = `hypervibes:chart-panel:${id}`;
  useEffect(() => {
    try {
      const saved: unknown = JSON.parse(localStorage.getItem(preferences) ?? "null");
      if (saved && typeof saved === "object") {
        if ("open" in saved && typeof saved.open === "boolean") setChatOpen(saved.open);
        // Old bottom-pane height preferences intentionally do not become widths.
        if ("width" in saved && typeof saved.width === "number" && Number.isFinite(saved.width)) setChatWidth(Math.max(minChatWidth, Math.min(maxChatWidth, saved.width)));
      }
    } catch { /* Optional preference. */ }
  }, [preferences]);
  function panelPreference(open: boolean, width: number) {
    setChatOpen(open); setChatWidth(width);
    try { localStorage.setItem(preferences, JSON.stringify({ open, width })); } catch { /* Optional preference. */ }
  }
  const selectedObject = layout?.document.drawings.find((d) => d.id === selected && d.coin === layout.document.coin);
  const objects = layout?.document.drawings.filter((d) => d.coin === layout.document.coin) ?? [];
  const allLocked = objects.length > 0 && objects.every((d) => d.locked);
  const timeframeHidden = layout ? objects.filter((d) => !d.hidden && d.intervals.length > 0 && !d.intervals.includes(layout.document.interval)) : [];
  async function save(drawing: ChartDrawing) { return (await change("edit", { label: `Edit ${drawing.name}`, upsert: [drawing] }))?.revision; }
  function selectLayout(value: string) { choose(value); setSelected(""); setTool("select"); setEditing(false); }
  function layoutAction(action: LayoutAction) {
    if (action === "notes") setNotesOpen(true);
    else if (action === "delete") { if (layout && window.confirm(`Delete saved layout “${layout.document.name}”? Its chat messages remain available.`)) void remove(); }
    else setNameDialog(action);
  }
  function onKey(event: KeyboardEvent<HTMLElement>) {
    if (event.target instanceof HTMLElement && (event.target.matches("input,textarea,select") || event.target.isContentEditable) || document.querySelector("dialog[open]")) return;
    if (event.key === "Escape" && tool !== "select") { event.preventDefault(); event.stopPropagation(); setTool("select"); return; }
    const key = event.key.toLowerCase();
    if ((event.ctrlKey || event.metaKey) && (key === "z" || key === "y")) {
      event.preventDefault();
      const redo = key === "y" || event.shiftKey;
      if (!pending && (redo ? layout?.canRedo : layout?.canUndo)) void change(redo ? "redo" : "undo");
    }
    if ((event.key === "Delete" || event.key === "Backspace") && selectedObject && !selectedObject.locked && !pending) { event.preventDefault(); void change("edit", { label: `Delete ${selectedObject.name}`, remove: [selectedObject.id] }); }
  }
  if (!instruments.length) return <p role="alert" className="text-amber-200">No perpetual instruments are available.</p>;
  return <section ref={workspace} aria-label="Trading chart" onKeyDown={onKey} className="flex h-[calc(100dvh-6rem)] min-h-[480px] w-full min-w-0 flex-col gap-2">
    <h2 className="sr-only">Chart workspace</h2>
    <div role="toolbar" aria-label="Chart controls" className="flex shrink-0 flex-nowrap items-center gap-1.5 overflow-x-auto pb-1">
      <select aria-label="Layout" value={layout?.id ?? ""} disabled={pending} onChange={(e) => selectLayout(e.target.value)} className={`${button} h-8 w-40 shrink-0 truncate bg-zinc-950`}>
        {!layouts.length && <option value="">No layouts</option>}{layouts.map((l) => <option key={l.id} value={l.id}>{l.document.name}</option>)}
      </select>
      <ChartLayoutSettings pending={pending} hasLayout={Boolean(layout)} onAction={layoutAction} />
      <SingleInstrumentPicker key={layout?.id} instruments={instruments} value={layout?.document.coin ?? null} disabled={pending || !layout} saveError={error} className={`${control} h-8 shrink-0 px-2`} onChange={async (coin) => {
        if (!layout) return false;
        if (coin === layout.document.coin) return true;
        const saved = await change("edit", { label: "Change instrument", settings: { coin } });
        if (!saved) return false;
        setSelected(""); setTool("select"); setEditing(false);
        return true;
      }} />
      <select aria-label="Timeframe" value={layout?.document.interval ?? ""} disabled={pending || !layout} onChange={(e) => { if (isChartInterval(e.target.value)) { setTool("select"); void change("edit", { label: "Change timeframe", settings: { interval: e.target.value } }); } }} className={`${button} h-8 w-20 shrink-0 bg-zinc-950`}>
        {!layout && <option value="">Timeframe</option>}{chartIntervals.map((interval) => <option key={interval}>{interval}</option>)}
      </select>
      <button type="button" className={iconButton} aria-label="Account overlays" title="Account overlays · Trades, position and orders (read-only)" disabled={!layout} onClick={() => setOverlayDialog(true)}><ChartIcon name="account" /></button>
      <button type="button" className={iconButton} aria-label="Magnet" aria-pressed={layout?.document.magnet ?? false} disabled={pending || !layout} title={`${layout?.document.magnet ? "Magnet on" : "Magnet off"} · Snap to candle OHLC · Ctrl/Cmd temporarily inverts snapping`} onClick={() => { if (layout) void change("edit", { label: "Toggle magnet", settings: { magnet: !layout.document.magnet } }); }}><ChartIcon name="magnet" /></button>
      <button type="button" className={iconButton} aria-label={layout?.document.hideDrawings ? "Show drawings" : "Hide drawings"} aria-pressed={Boolean(layout && !layout.document.hideDrawings)} disabled={pending || !layout} title={layout?.document.hideDrawings ? "Show drawings" : "Hide drawings"} onClick={() => { if (layout) void change("edit", { label: "Toggle drawings", settings: { hideDrawings: !layout.document.hideDrawings } }); }}><ChartIcon name={layout?.document.hideDrawings ? "hidden" : "eye"} /></button>
      <button type="button" className={iconButton} aria-label={toolsOpen ? "Hide annotation tools" : "Show annotation tools"} aria-pressed={toolsOpen} aria-expanded={toolsOpen} disabled={!layout} title={toolsOpen ? "Hide annotation tools" : "Show annotation tools"} onClick={() => { setToolsOpen(!toolsOpen); setTool("select"); }}><ChartIcon name="tools" /></button>
      <button type="button" className={iconButton} aria-label={allLocked ? "Unlock objects" : "Lock objects"} aria-pressed={allLocked} disabled={pending || !objects.length} title={`${allLocked ? "Unlock" : "Lock"} all annotations for ${layout?.document.coin ?? "this instrument"}`} onClick={() => { void change("edit", { label: allLocked ? "Unlock drawings" : "Lock drawings", upsert: objects.filter((d) => d.locked === allLocked).map((d) => ({ ...d, locked: !allLocked })) }); }}><ChartIcon name={allLocked ? "lock" : "unlock"} /></button>
      <button type="button" className={iconButton} aria-label={chatOpen ? "Hide chat" : "Show chat"} aria-pressed={chatOpen} aria-expanded={chatOpen} title={chatOpen ? "Hide chat" : "Show chat"} onClick={() => panelPreference(!chatOpen, splitWidth)}><ChartIcon name={chatOpen ? "chat" : "chatHidden"} /></button>
      <div ref={setChartStatusHost} aria-label="Chart status" className="ml-auto flex h-8 shrink-0 items-center pl-2" />
    </div>
    <span role="status" className="sr-only">{pending ? "Saving…" : error ? "Not saved · see error" : layout ? "Saved" : ready ? "Create a layout to begin" : "Loading…"}</span>
    {error && <p role="alert" className="text-sm text-amber-200">{error}</p>}
    {layout && !layout.document.hideDrawings && timeframeHidden.length > 0 && <p role="status" className="flex flex-wrap items-center gap-2 text-xs text-amber-200">
      <span>{timeframeHidden.length} {timeframeHidden.length === 1 ? "drawing" : "drawings"} hidden on {layout.document.interval} by timeframe settings.</span>
      <button type="button" className={button} disabled={pending || timeframeHidden.every((d) => d.locked)} title="Remove timeframe restrictions from these unlocked drawings. Locked drawings are unchanged." onClick={() => { void change("edit", { label: "Show drawings on all timeframes", upsert: timeframeHidden.filter((d) => !d.locked).map((d) => ({ ...d, intervals: [] })) }); }}>Show on all timeframes</button>
    </p>}
    <div aria-label="Chart and chat workspace" className="flex min-h-0 flex-1 overflow-x-auto">
      <div className="flex min-w-[280px] flex-1 gap-2">
        {layout && toolsOpen && <div role="toolbar" aria-label="Drawing tools" className="flex w-11 shrink-0 flex-col gap-1 overflow-y-auto">
          {(["select", ...drawingKinds] as const).map((kind) => <button key={kind} type="button" aria-label={toolLabels[kind]} title={`${toolLabels[kind]}${kind === "select" ? "" : " · click to place anchors"}`} aria-pressed={tool === kind} disabled={pending} onClick={() => { setTool(kind); setSelected(""); }} className={`${control} py-1.5 text-lg ${tool === kind ? "border-violet-500 bg-violet-950/50 text-violet-200" : ""}`}>{toolIcons[kind]}</button>)}
          <button type="button" className={`${control} py-1.5`} aria-label="Keep drawing" aria-pressed={keepDrawing} title="Keep drawing after placing an object" onClick={() => setKeepDrawing(!keepDrawing)}>∞</button>
        </div>}
        {layout ? <div ref={canvas} tabIndex={0} aria-label="Chart canvas" onPointerDown={(e) => { if (!(e.target instanceof Element) || !e.target.closest("button,input,textarea,select")) e.currentTarget.focus(); }} className="flex min-w-0 flex-1 flex-col outline-none focus-visible:ring-1 focus-visible:ring-violet-500">
          <MarketChart key={`${layout.id}/${layout.document.coin}/${layout.document.interval}`} ref={chart} id={id} layout={layout} tool={tool} selected={selected} disabled={pending} statusHost={chartStatusHost} overlayVisibility={overlays} onSelect={(drawingId) => { setSelected(drawingId); canvas.current?.focus({ preventScroll: true }); }} onSave={save} onDrawn={() => { if (!keepDrawing) setTool("select"); }} onEdit={() => setEditing(true)} />
          {(toolsOpen || selectedObject) && <div className="flex min-h-8 flex-wrap items-center gap-2 py-1 text-xs text-zinc-500">
            {toolsOpen && <span>{toolLabels[tool]}{tool !== "select" ? " · Click anchors · Esc cancels" : " · Select an object or draw from the toolbar"}</span>}
            {selectedObject && <>
              <span className="text-violet-300">{selectedObject.name}</span>
              <button className={button} disabled={pending || selectedObject.locked} onClick={() => setEditing(true)}>Settings</button>
              <button className={button} disabled={pending} onClick={() => { void save({ ...selectedObject, locked: !selectedObject.locked }); }}>{selectedObject.locked ? "Unlock" : "Lock"}</button>
              <button className={button} disabled={pending || selectedObject.locked} onClick={() => { void change("edit", { label: `Delete ${selectedObject.name}`, remove: [selectedObject.id] }); }}>Delete</button>
            </>}
          </div>}
        </div> : <p className="p-4 text-sm text-zinc-500">Create a layout from the layout settings to begin.</p>}
      </div>
      <aside hidden={!chatOpen} aria-label="Chart chat panel" className="min-h-0 shrink-0" style={{ width: splitWidth + 12 }}>
        <div className="flex h-full min-h-0">
          <div role="separator" aria-label="Resize chart chat" aria-orientation="vertical" aria-valuemin={minChatWidth} aria-valuemax={chatLimit} aria-valuenow={splitWidth} aria-valuetext={`${splitWidth}px chat width`} tabIndex={0}
            className="flex w-3 shrink-0 cursor-col-resize touch-none items-center justify-center rounded hover:bg-zinc-800 focus-visible:ring-1 focus-visible:ring-violet-500"
            onKeyDown={(e) => {
              const delta = e.key === "ArrowLeft" ? 20 : e.key === "ArrowRight" ? -20 : 0;
              if (delta || e.key === "Home" || e.key === "End") { e.preventDefault(); panelPreference(true, e.key === "Home" ? minChatWidth : e.key === "End" ? chatLimit : clampWidth(splitWidth + delta)); }
            }}
            onPointerDown={(e) => { if (e.button !== 0) return; e.preventDefault(); resize.current = { x: e.clientX, width: splitWidth, pointerId: e.pointerId }; e.currentTarget.setPointerCapture(e.pointerId); }}
            onPointerMove={(e) => { const moving = resize.current; if (moving && moving.pointerId === e.pointerId) setChatWidth(clampWidth(moving.width + moving.x - e.clientX)); }}
            onPointerUp={(e) => { const moving = resize.current; if (!moving || moving.pointerId !== e.pointerId) return; resize.current = null; if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId); panelPreference(true, clampWidth(moving.width + moving.x - e.clientX)); }}
            onPointerCancel={(e) => { const moving = resize.current; if (moving && moving.pointerId === e.pointerId) { setChatWidth(moving.width); resize.current = null; if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId); } }}
            onLostPointerCapture={(e) => { const moving = resize.current; if (moving && moving.pointerId === e.pointerId) { setChatWidth(moving.width); resize.current = null; } }}><span className="h-12 w-1 rounded bg-zinc-700" /></div>
          <div className="min-h-0 min-w-0 flex-1"><AgentChatPage initial={initialChat} choices={choices} embedded visible={chatOpen}
            chartContext={layout ? { label: `${layout.document.name} · ${layout.document.coin} · ${layout.document.interval}${tool === "select" ? "" : " · Finish drawing or choose Select before sending"}`, disabled: pending || tool !== "select",
              capture: async (image) => { if (!chart.current) throw new Error("Chart is not ready"); return chart.current.capture(layout, selectedObject?.id ?? "", image); } } : undefined} /></div>
        </div>
      </aside>
    </div>
    {editing && selectedObject && layout && <ChartDrawingEditor key={`${layout.id}/${layout.revision}/${selected}`} value={selectedObject} pending={pending} saveError={error} onCancel={() => setEditing(false)} onApply={async (drawing) => (await save(drawing)) !== undefined} />}
    {overlayDialog && <ChartOverlayDialog value={overlays} onCancel={() => setOverlayDialog(false)} onApply={(value) => {
      setOverlays(value); setOverlayDialog(false);
      try { localStorage.setItem(overlayPreferences, JSON.stringify(value)); } catch { /* Optional preference. */ }
    }} />}
    {notesOpen && layout && <LayoutNotesDialog key={layout.id} notes={layout.document.notes} pending={pending} error={error} onCancel={() => setNotesOpen(false)} onApply={async (notes) => { if (await change("edit", { label: "Update layout notes", settings: { notes } })) setNotesOpen(false); }} />}
    {nameDialog && <LayoutNameDialog key={nameDialog} title={nameDialog === "new" ? "New layout" : nameDialog === "duplicate" ? "Duplicate layout" : "Rename layout"} initial={nameDialog === "new" ? "My chart" : `${layout?.document.name ?? "Chart"}${nameDialog === "duplicate" ? " copy" : ""}`} pending={pending} error={error} onCancel={() => setNameDialog(null)} onApply={async (name, operationId) => {
      const result = nameDialog === "rename" ? await change("edit", { label: "Rename layout", settings: { name } }) : await create(name, nameDialog === "duplicate", operationId);
      if (result) { setNameDialog(null); setSelected(""); }
    }} />}
  </section>;
}
function ChartOverlayDialog({ value, onApply, onCancel }: { value: ChartOverlayVisibility; onApply: (value: ChartOverlayVisibility) => void; onCancel: () => void }) {
  const [draft, setDraft] = useState(value);
  return <ConfigDialog title="Account overlays" onCancel={onCancel} onApply={() => onApply(draft)}>
    <div className="space-y-3 text-sm">{([["trades", "Historical buys & sells"], ["position", "Current position"], ["orders", "Open orders & TP/SL triggers"]] as const).map(([key, label]) => <label key={key} className="flex cursor-pointer items-center gap-3"><input type="checkbox" checked={draft[key]} onChange={(event) => setDraft({ ...draft, [key]: event.target.checked })} className="cursor-pointer accent-violet-500" />{label}</label>)}</div>
    <p className="mt-4 text-xs text-zinc-500">Read-only data from this Agent’s linked Hyperliquid account, including manual trades. History follows the journal. Visibility is saved on this device, independently of layouts and annotations.</p>
  </ConfigDialog>;
}
function LayoutNotesDialog({ notes, pending, error, onApply, onCancel }: { notes: string; pending: boolean; error: string; onApply: (notes: string) => Promise<void>; onCancel: () => void }) {
  const [value, setValue] = useState(notes);
  return <ConfigDialog title="Layout notes" applyLabel="Save notes" applyDisabled={pending} onCancel={onCancel} onApply={() => { void onApply(value); }}>
    <label className="block space-y-2 text-sm">Notes<textarea autoFocus rows={6} maxLength={8000} className={configInput} value={value} disabled={pending} onChange={(e) => setValue(e.target.value)} /></label>
    <p className="text-xs text-zinc-500">Local to this layout. Not published research or Agent-wide insights.</p>
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
  </ConfigDialog>;
}
function LayoutNameDialog({ title, initial, pending, error, onApply, onCancel }: { title: string; initial: string; pending: boolean; error: string; onApply: (name: string, operationId: string) => Promise<void>; onCancel: () => void }) {
  const [name, setName] = useState(initial);
  const request = useRef({ name: initial, id: crypto.randomUUID() });
  return <ConfigDialog title={title} onCancel={onCancel} onApply={() => { if (request.current.name !== name.trim()) request.current = { name: name.trim(), id: crypto.randomUUID() }; void onApply(name.trim(), request.current.id); }} applyDisabled={pending || !name.trim()}><label className="block space-y-2 text-sm">Name<input autoFocus maxLength={80} value={name} disabled={pending} onChange={(e) => setName(e.target.value)} className={configInput} /></label><p className="mt-3 text-xs text-zinc-500">Layouts autosave instrument, timeframe, objects, styles and notes. Reopening follows the latest candles.</p>{error && <p role="alert" className="text-sm text-red-300">{error}</p>}</ConfigDialog>;
}

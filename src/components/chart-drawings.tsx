import { useEffect, useRef, useState, type PointerEvent, type RefObject } from "react";
import { newDrawing, visibleDrawing, type ChartDocument, type ChartDrawing, type ChartPoint, type DrawingKind } from "../charts/contracts.js";
import { lineEnds, type ScreenPoint } from "../charts/geometry.js";

export interface ChartProjection { width: number; height: number; x: (time: number) => number | null; y: (price: number) => number | null; point: (x: number, y: number, snap: boolean) => ChartPoint | null }
export function ChartDrawings({ document, revision, projection, tool, selected, disabled, svgRef, onSelect, onSave, onDrawn, onEdit }: {
  document: ChartDocument; revision: number; projection: ChartProjection; tool: DrawingKind | "select"; selected: string; disabled: boolean; svgRef: RefObject<SVGSVGElement | null>;
  onSelect: (id: string) => void; onSave: (drawing: ChartDrawing) => Promise<number | undefined>; onDrawn: () => void; onEdit: () => void;
}) {
  const [preparationError, setPreparationError] = useState("");
  const [anchor, setAnchor] = useState<ChartPoint | null>(null);
  const [preview, setPreview] = useState<ChartPoint | null>(null);
  const [draft, setDraft] = useState<{ drawing: ChartDrawing; revision: number | null } | null>(null);
  const drag = useRef<{ drawing: ChartDrawing; handle: number | null; start: ChartPoint; draft: ChartDrawing | null; pointerId: number } | null>(null);
  const committedRevision = draft?.revision;
  const blocked = disabled || (committedRevision != null && revision < committedRevision);
  useEffect(() => {
    if (committedRevision != null && revision >= committedRevision) {
      setDraft((current) => current?.revision === committedRevision ? null : current);
    }
  }, [revision, committedRevision]);
  const screen = (point: ChartPoint): ScreenPoint | null => {
    const x = projection.x(point.time), y = projection.y(point.price);
    return x === null || y === null ? null : { x, y };
  };
  function location(event: PointerEvent<SVGElement>): ChartPoint | null {
    const bounds = svgRef.current!.getBoundingClientRect();
    return projection.point(Math.max(0, Math.min(projection.width, event.clientX - bounds.left)), Math.max(0, Math.min(projection.height, event.clientY - bounds.top)), document.magnet !== (event.ctrlKey || event.metaKey));
  }
  function start(event: PointerEvent<SVGElement>, drawing: ChartDrawing, handle: number | null) {
    if (tool !== "select" || event.button !== 0) return;
    event.preventDefault(); event.stopPropagation();
    if (blocked || drag.current) return;
    onSelect(drawing.id);
    if (drawing.locked) return;
    const point = location(event);
    if (!point) return;
    svgRef.current!.setPointerCapture(event.pointerId);
    drag.current = { drawing, handle, start: point, draft: null, pointerId: event.pointerId };
  }
  function move(event: PointerEvent<SVGSVGElement>) {
    if (drag.current && drag.current.pointerId !== event.pointerId) return;
    const point = location(event);
    if (!point) return;
    if (anchor) setPreview(point);
    const moving = drag.current;
    if (!moving) return;
    const points = moving.drawing.points.map((old, index) => moving.handle === index ? point : moving.handle !== null ? old : { time: Math.max(0, old.time + point.time - moving.start.time), price: Math.max(1e-12, old.price + point.price - moving.start.price) });
    moving.draft = { ...moving.drawing, points };
    setDraft({ drawing: moving.draft, revision: null });
  }
  async function persist(drawing: ChartDrawing) {
    let savedRevision: number | undefined;
    try { savedRevision = await onSave(drawing); }
    catch { /* The workspace reports save failures; discard this optimistic move. */ }
    setDraft((current) => current?.drawing !== drawing ? current : savedRevision === undefined ? null : { drawing, revision: savedRevision });
  }
  function finish(event: PointerEvent<SVGSVGElement>) {
    const moving = drag.current;
    if (!moving || moving.pointerId !== event.pointerId) return;
    drag.current = null;
    // Release before saving can disable interaction. The resulting lost-capture
    // event must not cancel the completed move or discard its optimistic preview.
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    if (moving.draft) void persist(moving.draft);
  }
  function cancel(event: PointerEvent<SVGSVGElement>) {
    if (!drag.current || drag.current.pointerId !== event.pointerId) return;
    drag.current = null; setDraft(null);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
  }
  function place(event: PointerEvent<SVGSVGElement>) {
    if (blocked || event.button !== 0) return;
    const point = location(event);
    if (!point) return;
    if (tool === "select") { onSelect(""); return; }
    const single = ["horizontal", "horizontal-ray", "vertical", "text"].includes(tool);
    if (!single && !anchor) { setAnchor(point); setPreview(point); return; }
    try {
      const drawing = newDrawing(tool, document.coin, single ? [point] : [anchor!, point]);
      setPreparationError("");
      if (["text", "callout"].includes(tool)) drawing.text = "Note";
      void onSave(drawing); onSelect(drawing.id); setAnchor(null); setPreview(null); onDrawn();
    } catch (cause) { setPreparationError(cause instanceof Error ? cause.message : "Unable to prepare the drawing"); }
  }
  const previewDrawing = draft && (draft.revision === null || revision < draft.revision) ? draft.drawing : null;
  const drawings = document.drawings.filter((d) => visibleDrawing(d, document)).map((d) => previewDrawing?.id === d.id ? previewDrawing : d);
  if (anchor && preview && tool !== "select") drawings.push({ ...newDrawing(tool, document.coin, [anchor, preview], "preview"), color: "#71717a" });
  return <><svg ref={svgRef} xmlns="http://www.w3.org/2000/svg" width={projection.width} height={projection.height} viewBox={`0 0 ${projection.width} ${projection.height}`} aria-label="Chart drawings"
    className="absolute left-0 top-0 z-10 overflow-hidden" style={{ pointerEvents: tool === "select" || blocked ? "none" : "auto", cursor: tool === "select" ? "default" : "crosshair", touchAction: "none" }}
    onPointerDown={place} onPointerMove={move} onPointerUp={finish} onPointerCancel={cancel} onLostPointerCapture={cancel}>
    {tool !== "select" && !blocked && <rect data-drawing-surface="true" width={projection.width} height={projection.height} fill="transparent" style={{ pointerEvents: "all" }} />}
    {drawings.map((drawing) => {
      const a = screen(drawing.points[0]!), b = screen(drawing.points[1] ?? drawing.points[0]!);
      if (!a || !b) return null;
      const [from, to] = lineEnds(drawing.kind, a, b, projection.width);
      const stroke = { stroke: drawing.color, strokeWidth: drawing.width, strokeDasharray: drawing.dashed ? "6 4" : undefined, fill: "none" };
      const path = drawing.kind === "vertical" ? `M ${a.x} 0 L ${a.x} ${projection.height}` : `M ${from.x} ${from.y} L ${to.x} ${to.y}`;
      const rectangle = drawing.kind === "rectangle";
      const text = drawing.kind === "text";
      const labelPoint = drawing.kind === "callout" ? b : a;
      const active = selected === drawing.id;
      return <g key={drawing.id} aria-label={drawing.name} onDoubleClick={(event) => { if (tool === "select" && !drawing.locked && !blocked) { event.stopPropagation(); onSelect(drawing.id); onEdit(); } }}>
        <title>{drawing.name}{drawing.locked ? " (locked)" : ""} · {drawing.author === "agent" ? "Agent" : "You"}</title>
        {rectangle ? <rect x={Math.min(a.x, b.x)} y={Math.min(a.y, b.y)} width={Math.abs(b.x - a.x)} height={Math.abs(b.y - a.y)} {...stroke} fill={drawing.color} fillOpacity={drawing.fillOpacity} /> : !text ? <path d={path} {...stroke} /> : null}
        {drawing.kind === "arrow" && <path d={`M ${b.x - 12 * Math.cos(Math.atan2(b.y - a.y, b.x - a.x) - 0.45)} ${b.y - 12 * Math.sin(Math.atan2(b.y - a.y, b.x - a.x) - 0.45)} L ${b.x} ${b.y} L ${b.x - 12 * Math.cos(Math.atan2(b.y - a.y, b.x - a.x) + 0.45)} ${b.y - 12 * Math.sin(Math.atan2(b.y - a.y, b.x - a.x) + 0.45)}`} {...stroke} />}
        {(drawing.kind === "horizontal" || drawing.kind === "horizontal-ray") && <text x={projection.width - 6} y={a.y - 5} textAnchor="end" fill={drawing.color} fontSize="11" fontFamily="sans-serif" stroke="#0d0d10" strokeWidth="3" paintOrder="stroke">{drawing.points[0]!.price.toLocaleString("en", { maximumSignificantDigits: 10 })}</text>}
        {drawing.text && <text x={labelPoint.x + 6} y={labelPoint.y - 8} fill={drawing.color} fontSize="13" fontFamily="sans-serif" stroke="#0d0d10" strokeWidth="3" paintOrder="stroke">{drawing.text.split("\n").map((line, i) => <tspan key={i} x={labelPoint.x + 6} dy={i ? "1.2em" : 0}>{line}</tspan>)}</text>}
        {tool === "select" && (rectangle ? <rect x={Math.min(a.x, b.x)} y={Math.min(a.y, b.y)} width={Math.abs(b.x - a.x)} height={Math.abs(b.y - a.y)} fill="transparent" stroke="transparent" strokeWidth="12" style={{ pointerEvents: "all", cursor: blocked ? "progress" : drawing.locked ? "pointer" : "move" }} onPointerDown={(event) => start(event, drawing, null)} /> : text ? <rect x={a.x} y={a.y - 24} width={Math.max(60, drawing.text.length * 8)} height="30" fill="transparent" style={{ pointerEvents: "all", cursor: blocked ? "progress" : drawing.locked ? "pointer" : "move" }} onPointerDown={(event) => start(event, drawing, null)} /> : <path d={path} fill="none" stroke="transparent" strokeWidth="14" style={{ pointerEvents: "stroke", cursor: blocked ? "progress" : drawing.locked ? "pointer" : "move" }} onPointerDown={(event) => start(event, drawing, null)} />)}
        {active && drawing.points.map((p, index) => {
          const position = screen(p);
          return position ? <circle key={index} data-handle="true" cx={position.x} cy={position.y} r="5" fill="#0d0d10" stroke={drawing.color} strokeWidth="2" style={{ pointerEvents: tool === "select" ? "all" : "none", cursor: blocked ? "progress" : drawing.locked ? "not-allowed" : "grab" }} onPointerDown={(event) => start(event, drawing, index)} /> : null;
        })}
      </g>;
    })}
  </svg>{preparationError && <p role="alert" className="absolute left-2 top-2 z-20 text-sm text-red-300">{preparationError}</p>}</>;
}

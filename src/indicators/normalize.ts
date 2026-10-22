import type { IndicatorAnchor, IndicatorDrawing, IndicatorFill, IndicatorLevel, IndicatorMarker, IndicatorOutput, IndicatorVisualStyle, IndicatorVmRequest, NormalizedIndicator } from "./contracts.js";

/** Self-contained trusted function, serialized into QuickJS. No host capabilities or module captures. */
export function normalizePineContext(raw: unknown, args: IndicatorVmRequest): Omit<NormalizedIndicator, "inputs" | "resolvedInputs"> {
  type Row = Record<string, unknown>;
  const row = (value: unknown): Row => {
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Malformed PineTS output object");
    return value as Row;
  };
  const array = (value: unknown): unknown[] => { if (!Array.isArray(value)) throw new Error("Malformed PineTS output collection"); return value; };
  const num = (value: unknown): number => { if (typeof value !== "number" || !Number.isFinite(value)) throw new Error("Non-finite PineTS coordinate/value"); return value; };
  const integer = (value: unknown): number => { const n = num(value); if (!Number.isSafeInteger(n)) throw new Error("Invalid PineTS integer"); return n; };
  const text = (value: unknown, fallback = ""): string => { if (value === undefined || value === null) return fallback; if (typeof value !== "string") throw new Error("Invalid PineTS text"); return value; };
  const optionalText = (value: unknown): string | undefined => value === undefined || value === null ? undefined : text(value);
  const numberOrNull = (value: unknown): number | null => value === null || value === undefined || (typeof value === "number" && Number.isNaN(value)) ? null : num(value);
  const context = row(raw), declaration = row(context.indicator), plots = row(context.plots);
  const overlay = declaration.overlay === true;
  const outputs: IndicatorOutput[] = [], levels: IndicatorLevel[] = [], fills: IndicatorFill[] = [], markers: IndicatorMarker[] = [], drawings: IndicatorDrawing[] = [];
  const times = new Set(args.candles.map((c) => c.openTime / 1000));
  const timeIndexes = new Map(args.candles.map((c, index) => [c.openTime / 1000, index]));
  const time = (value: unknown) => { const t = num(value) / 1000; if (!times.has(t)) throw new Error("Plot time is outside the input candles"); return t; };
  const pane = (options: Row) => ({ overlay: options.overlay === true || (options.overlay === undefined && overlay), forceOverlay: options.force_overlay === true });
  const style = (options: Row): IndicatorVisualStyle => ({
    color: optionalText(options.color), width: options.linewidth === undefined ? undefined : num(options.linewidth),
    lineStyle: optionalText(options.linestyle), hidden: options.display === 0 || options.display === "none",
    offset: options.offset === undefined ? 0 : integer(options.offset), showLast: options.show_last === undefined ? undefined : integer(options.show_last),
  });
  const anchor = (x: unknown, y: unknown, xloc: unknown): IndicatorAnchor => {
    const price = num(y);
    if (xloc === "bt" || xloc === "bar_time") return { time: num(x) / 1000, price };
    if (xloc !== "bi" && xloc !== "bar_index" && xloc !== undefined) throw new Error(`Unsupported drawing xloc: ${String(xloc)}`);
    const barIndex = integer(x), candle = args.candles[barIndex];
    const first = args.candles[0];
    if (!first) throw new Error("Drawing requires input candles");
    return { time: (candle?.openTime ?? first.openTime + barIndex * args.intervalMs) / 1000, price, barIndex };
  };
  const extension = (value: unknown): "none" | "left" | "right" | "both" => {
    if (value === "r" || value === "right") return "right";
    if (value === "l" || value === "left") return "left";
    if (value === "b" || value === "both") return "both";
    if (value === undefined || value === "n" || value === "none") return "none";
    throw new Error(`Unsupported drawing extension: ${String(value)}`);
  };
  let markerOutputs = 0;
  for (const [id, rawPlot] of Object.entries(plots)) {
    if (["__proto__", "constructor", "prototype"].includes(id)) throw new Error("Unsafe PineTS plot ID");
    const plot = row(rawPlot), data = array(plot.data), options = row(plot.options ?? {});
    if (!id.startsWith("__") && data.length > args.limits.candles) throw new Error("Output point limit exceeded");
    if (id.startsWith("__")) {
      const objects = data.flatMap((point) => array(row(point).value));
      const live = objects.map(row).filter((object) => object._deleted !== true);
      if (!["__lines__", "__boxes__", "__labels__"].includes(id)) {
        if (live.length) throw new Error(`Unsupported nonempty PineTS output: ${id}`);
        continue;
      }
      for (const object of live) {
        if (drawings.length >= args.limits.drawings) throw new Error("Generated drawing limit exceeded");
        const base = { id: `${id}:${String(object.id)}`, ...pane({ ...options, force_overlay: object.force_overlay }),
          color: optionalText(object.color ?? object.border_color), width: object.width === undefined && object.border_width === undefined ? undefined : num(object.width ?? object.border_width),
          lineStyle: optionalText(object.style ?? object.border_style), extend: extension(object.extend) };
        if (id === "__lines__") drawings.push({ ...base, kind: "line", from: anchor(object.x1, object.y1, object.xloc), to: anchor(object.x2, object.y2, object.xloc) });
        else if (id === "__boxes__") drawings.push({ ...base, kind: "box", from: anchor(object.left, object.top, object.xloc), to: anchor(object.right, object.bottom, object.xloc), background: optionalText(object.bgcolor), text: text(object.text), textColor: optionalText(object.text_color), size: optionalText(object.text_size) });
        else drawings.push({ ...base, kind: "label", at: anchor(object.x, object.y, object.xloc), text: text(object.text), textColor: optionalText(object.textcolor), size: optionalText(object.size), shape: optionalText(object.style), location: text(object.yloc, "pr") });
      }
      continue;
    }
    const title = text(plot.title, id), kind = text(options.style, "style_line"), settings = style(options);
    const visible = (t: number) => !settings.hidden && (settings.showLast === undefined || timeIndexes.get(t)! >= args.candles.length - settings.showLast);
    if (kind === "fill") {
      fills.push({ id, title, from: text(plot.plot1 ?? options.plot1), to: text(plot.plot2 ?? options.plot2), color: optionalText(options.color), hidden: settings.hidden,
        points: data.flatMap((item) => { const p = row(item), opts = row(p.options ?? {}), t = time(p.time); return visible(t) ? [{ time: t, color: optionalText(opts.color ?? options.color) }] : []; }) });
      continue;
    }
    if (kind === "hline") {
      const values = data.map((p) => numberOrNull(row(p).value)).filter((v) => v !== null);
      if (values.length && values.some((v) => v !== values[0])) throw new Error("Horizontal level changed within calculation");
      if (values.length) levels.push({ id, title, value: values[0]!, ...pane(options), style: settings });
      continue;
    }
    if (["shape", "char", "arrow"].includes(kind)) {
      if (++markerOutputs > args.limits.markerOutputs) throw new Error("Marker output limit exceeded");
      const markerKind = text(plot._callsiteId).startsWith("plotarrow") || kind === "arrow" ? "arrow" : kind === "char" ? "char" : "shape";
      for (const [index, item] of data.entries()) {
        const p = row(item), opts = { ...options, ...row(p.options ?? {}) }, value = p.value;
        if (value === null || value === undefined || value === false || value === 0 || (typeof value === "number" && Number.isNaN(value))) continue;
        if (typeof value !== "boolean") num(value);
        const pointSettings = style(opts), sourceTime = time(p.time);
        if (!visible(sourceTime) || pointSettings.hidden) continue;
        if (markers.length >= args.limits.markerEvents) throw new Error("Marker event limit exceeded");
        markers.push({ id: `${id}:${index}`, outputId: id, kind: markerKind, time: sourceTime,
          displayTime: sourceTime + pointSettings.offset * args.intervalMs / 1000, offset: pointSettings.offset,
          value: value as number | boolean, ...pane(opts), location: text(opts.location, "AboveBar"), shape: optionalText(opts.shape),
          text: optionalText(opts.text), character: optionalText(opts.char), color: optionalText(opts.color), textColor: optionalText(opts.textcolor), size: optionalText(opts.size) });
      }
      continue;
    }
    if (!["style_line", "style_linebr", "style_histogram", "style_columns"].includes(kind)) {
      if (data.some((p) => { const v = row(p).value; return v !== null && v !== undefined && !(typeof v === "number" && Number.isNaN(v)); })) throw new Error(`Unsupported nonempty PineTS output style: ${kind}`);
      continue;
    }
    if (outputs.length >= args.limits.numericOutputs) throw new Error("Numeric output limit exceeded");
    const points = data.map((item) => {
      const p = row(item), opts = { ...options, ...row(p.options ?? {}) }, t = time(p.time);
      return { time: t, displayTime: t + style(opts).offset * args.intervalMs / 1000, value: numberOrNull(p.value), color: optionalText(opts.color) };
    });
    const finite = points.filter((p) => p.value !== null);
    outputs.push({ id, title, kind: kind === "style_histogram" ? "histogram" : kind === "style_columns" ? "columns" : "line", ...pane(options), style: settings,
      finiteCount: finite.length, firstFiniteTime: finite[0]?.time ?? null,
      points: points.map((p) => ({ ...p, ...(visible(p.time) ? {} : { color: "#00000000" }) })) });
  }
  if (outputs.length + levels.length > args.limits.numericOutputs) throw new Error("Numeric output limit exceeded");
  for (const fill of fills) if (![outputs, levels].flat().some((p) => p.id === fill.from) || ![outputs, levels].flat().some((p) => p.id === fill.to)) throw new Error("Fill references an unsupported output");
  markers.sort((a, b) => a.time - b.time || a.outputId.localeCompare(b.outputId) || a.id.localeCompare(b.id));
  const byTime = new Map(outputs.map((output) => [output.id, new Map(output.points.map((p) => [p.time, p.value]))]));
  const values = args.candles.map((c) => ({ time: c.openTime / 1000, value: Object.fromEntries(outputs.map((o) => [o.id, byTime.get(o.id)?.get(c.openTime / 1000) ?? null])) }));
  const warnings = array(context.warnings ?? []).map((w) => typeof w === "string" ? w : JSON.stringify(w));
  return { title: text(declaration.title), overlay, outputs, levels, fills, markers, drawings, values,
    warnings: warnings.slice(0, args.limits.warnings), warningsOmitted: Math.max(0, warnings.length - args.limits.warnings) };
}

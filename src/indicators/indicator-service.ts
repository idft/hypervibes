import { bodyHash, canonical } from "../canonical.js";
import type { SharedDatabase } from "../db/shared-database.js";
import type { MarketCharts } from "../hyperliquid/market-charts.js";
import { BuiltinIndicatorCatalog } from "./builtin-catalog.js";
import { fetchIndicatorWindow } from "./candle-window.js";
import { indicatorLimits, indicatorWrapperVersion, type IndicatorActor, type IndicatorDescriptor, type IndicatorPreviewRequest, type IndicatorResult, type IndicatorRunRequest, type IndicatorSaveRequest, type NormalizedIndicator } from "./contracts.js";
import { IndicatorStore, indicatorPage } from "./indicator-store.js";
import { IndicatorRuntime } from "./runtime.js";
import type { ChartLayout } from "../charts/contracts.js";
import { intervalMillis } from "../hyperliquid/chart-contracts.js";
import { IndicatorError, parseIndicatorId, resolveInputs, validateInputMigration, validateInputs, validateSource } from "./validation.js";

interface CatalogEvent { indicatorId: string; version: number; sourceHash: string }
interface CacheEntry { agentId: string; indicatorId: string; expires: number; bytes: number; value: IndicatorResult }
interface Job { agentId: string; controller: AbortController; done: Promise<void> }

/** Shared application service. No I/O, workers or market requests on import/construction. */
export class IndicatorService {
  private readonly store: IndicatorStore;
  private readonly catalog = new BuiltinIndicatorCatalog();
  private readonly cache = new Map<string, CacheEntry>();
  private cacheBytes = 0;
  private initialized = false;
  private initializing?: Promise<void>;
  private stopping = false;
  private closing?: Promise<void>;
  private readonly deleted = new Set<string>();
  private readonly jobs = new Set<Job>();
  private readonly listeners = new Map<string, Set<(event: CatalogEvent) => void>>();
  constructor(database: SharedDatabase, private readonly market: Pick<MarketCharts, "researchHistory">, private readonly runtime = new IndicatorRuntime(), private readonly reportError: (error: unknown) => void = () => {}) {
    this.store = new IndicatorStore(database);
  }
  initialize(): Promise<void> {
    if (this.stopping) return Promise.reject(new IndicatorError("Indicator service is closed", 503));
    return this.initializing ??= (async () => {
      try { await this.runtime.initialize(); await this.catalog.initialize(this.runtime); if (!this.stopping) this.initialized = true; }
      catch (error) { await this.runtime.close(); throw error; }
    })();
  }
  private async admit(agentId: string) {
    if (!this.initialized || this.stopping || this.deleted.has(agentId)) throw new IndicatorError("Indicator service is unavailable", 503);
    await this.store.assertAgent(agentId);
    if (this.stopping || this.deleted.has(agentId)) throw new IndicatorError("Indicator service is unavailable", 503);
  }
  private async work<T>(agentId: string, signal: AbortSignal | undefined, callback: (signal: AbortSignal) => Promise<T>): Promise<T> {
    signal?.throwIfAborted();
    await this.admit(agentId);
    if (this.stopping || this.deleted.has(agentId)) throw new IndicatorError("Indicator service is unavailable", 503);
    let settle!: () => void;
    const job: Job = { agentId, controller: new AbortController(), done: new Promise<void>((resolve) => { settle = resolve; }) };
    this.jobs.add(job);
    try { return await callback(signal ? AbortSignal.any([signal, job.controller.signal]) : job.controller.signal); }
    finally { this.jobs.delete(job); settle(); }
  }
  async list(agentId: string, options: { query?: string; kind?: "all" | "builtin" | "custom"; offset?: number; limit?: number } = {}) {
    await this.admit(agentId);
    const { offset, limit } = indicatorPage(options.offset, options.limit);
    if (options.query !== undefined && (typeof options.query !== "string" || options.query.length > 160)) throw new IndicatorError("Invalid indicator search");
    if (options.kind !== undefined && !["all", "builtin", "custom"].includes(options.kind)) throw new IndicatorError("Invalid indicator kind");
    const entries = [...(options.kind === "custom" ? [] : this.catalog.list()), ...(options.kind === "builtin" ? [] : await this.store.list(agentId))];
    const query = options.query?.toLowerCase() ?? "";
    const filtered = entries.filter((entry) => `${entry.name}\n${entry.description}`.toLowerCase().includes(query));
    const headers = filtered.slice(offset, offset + limit).map((entry) => ({ id: entry.id, builtIn: entry.builtIn, name: entry.name, description: entry.description, version: entry.version, sourceHash: entry.sourceHash, overlay: entry.overlay }));
    return { indicators: headers, total: filtered.length, nextOffset: offset + limit < filtered.length ? offset + limit : null };
  }
  private async resolve(agentId: string, id: string, version?: number) {
    const parsed = parseIndicatorId(id);
    return parsed.kind === "builtin" ? this.catalog.get(id, version) : this.store.get(agentId, id, version);
  }
  async get(agentId: string, id: string, options: { version?: number; includeSource?: boolean; sourceOffset?: number; sourceLimit?: number } = {}) {
    await this.admit(agentId);
    const entry = await this.resolve(agentId, id, options.version);
    const offset = options.sourceOffset ?? 0, limit = options.sourceLimit ?? 16_000;
    if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(limit) || limit < 1 || limit > 16_000) throw new IndicatorError("Invalid source page");
    return { descriptor: entry.descriptor, ...(options.includeSource ? { source: { text: entry.source.slice(offset, offset + limit), offset, total: entry.source.length, nextOffset: offset + limit < entry.source.length ? offset + limit : null } } : {}) };
  }
  async versions(agentId: string, id: string, options: { offset?: number; limit?: number } = {}) {
    await this.admit(agentId);
    const { offset, limit } = indicatorPage(options.offset, options.limit);
    if (parseIndicatorId(id).kind === "custom") return this.store.versions(agentId, id, { offset, limit });
    const { descriptor } = this.catalog.get(id);
    return { versions: offset === 0 ? [{ version: descriptor.version, sourceHash: descriptor.sourceHash }] : [], nextOffset: null };
  }
  private evict(key: string) { const entry = this.cache.get(key); if (entry) { this.cacheBytes -= entry.bytes; this.cache.delete(key); } }
  private clear(agentId: string, id?: string) {
    for (const [key, entry] of this.cache) if (entry.agentId === agentId && (!id || entry.indicatorId === id)) this.evict(key);
  }
  private async calculate(agentId: string, descriptor: IndicatorDescriptor, source: string, request: IndicatorPreviewRequest, signal: AbortSignal, sharedWindow?: Awaited<ReturnType<typeof fetchIndicatorWindow>>): Promise<IndicatorResult> {
    indicatorProjectionLimits(request);
    const overrides = validateInputs(request.inputs ?? {});
    const resolvedInputs = descriptor.builtIn ? this.catalog.resolve(descriptor.id, overrides) : resolveInputs(descriptor.inputs, overrides);
    const window = sharedWindow ?? await fetchIndicatorWindow(this.market, request.coin, request.interval, request.start, request.end, signal);
    signal.throwIfAborted();
    const candleHash = bodyHash(canonical({ coin: request.coin, interval: request.interval, intervalMs: window.width, candles: window.candles }));
    const inputSchemaHash = bodyHash(canonical(descriptor.inputs));
    const key = bodyHash(canonical({ agentId, id: descriptor.id, version: descriptor.version, sourceHash: descriptor.sourceHash, resolvedInputs, engine: this.runtime.engine, wrapper: indicatorWrapperVersion, candleHash, inputSchemaHash, seedStart: window.start, expected: window.expected, seedCoverage: window.seedCoverage }));
    const cached = this.cache.get(key);
    if (cached && cached.expires > Date.now()) {
      this.cache.delete(key); this.cache.set(key, cached);
      return { ...structuredClone(cached.value), start: request.start, end: window.end };
    }
    if (cached) this.evict(key);
    const calculation = await this.runtime.execute(agentId, { source, coin: request.coin, pineTimeframe: window.pineTimeframe, intervalMs: window.width, candles: window.candles, inputs: overrides }, signal);
    signal.throwIfAborted();
    // Descriptor schema is frozen with its source. Never cache a result under a different schema.
    if (canonical(calculation.inputs) !== canonical(descriptor.inputs)) throw new IndicatorError("Indicator input metadata changed unexpectedly");
    const result: IndicatorResult = { ...calculation, schemaVersion: 1, indicatorId: descriptor.id, builtIn: descriptor.builtIn, version: descriptor.version, sourceHash: descriptor.sourceHash,
      engine: this.runtime.engine, wrapperVersion: indicatorWrapperVersion, coin: request.coin, interval: request.interval, start: request.start, end: window.end,
      inputSchemaHash, computedAt: Date.now(), candleHash, resultHash: bodyHash(canonical(calculation)),
      dataset: { count: window.candles.length, firstOpenTime: window.candles[0]!.openTime / 1000, lastOpenTime: window.candles.at(-1)!.openTime / 1000 },
      outputDataset: { count: calculation.values.length, firstTime: calculation.values[0]?.time ?? null, lastTime: calculation.values.at(-1)?.time ?? null },
      coverage: { complete: window.seedCoverage === "complete", closedOnly: true, expected: window.expected, returned: window.candles.length, seedCoverage: window.seedCoverage } };
    const bytes = Buffer.byteLength(JSON.stringify(result));
    if (bytes > indicatorLimits.resultBytes) throw new IndicatorError("Full indicator result exceeds byte limit");
    for (const [oldKey, entry] of this.cache) if (entry.expires <= Date.now()) this.evict(oldKey);
    while (this.cache.size >= indicatorLimits.cacheEntries || this.cacheBytes + bytes > indicatorLimits.cacheBytes) this.evict(this.cache.keys().next().value!);
    this.cache.set(key, { agentId, indicatorId: descriptor.id, expires: Date.now() + indicatorLimits.cacheTtlMs, bytes, value: structuredClone(result) }); this.cacheBytes += bytes;
    return result;
  }
  run(agentId: string, request: IndicatorRunRequest, options: { signal?: AbortSignal } = {}) {
    return this.work(agentId, options.signal, async (signal) => {
      const entry = await this.resolve(agentId, request.indicatorId, request.version);
      return this.calculate(agentId, entry.descriptor, entry.source, { ...request, source: entry.source }, signal);
    });
  }
  preview(agentId: string, request: IndicatorPreviewRequest, options: { signal?: AbortSignal } = {}) {
    return this.work(agentId, options.signal, (signal) => this.previewSource(agentId, request, signal));
  }
  private async previewSource(agentId: string, request: IndicatorPreviewRequest, signal: AbortSignal) {
    validateSource(request.source);
    indicatorProjectionLimits(request);
    const window = await fetchIndicatorWindow(this.market, request.coin, request.interval, request.start, request.end, signal);
    const meta = await this.runtime.execute(agentId, { source: request.source, coin: request.coin, pineTimeframe: window.pineTimeframe, intervalMs: window.width, candles: window.candles, inputs: request.inputs ?? {} }, signal);
    // Preview already computed once: do not fetch or execute again just to attach provenance.
    return this.previewResult(meta, request, window);
  }
  private previewResult(meta: NormalizedIndicator, request: IndicatorPreviewRequest, window: Awaited<ReturnType<typeof fetchIndicatorWindow>>): IndicatorResult {
    const result: IndicatorResult = { ...meta, schemaVersion: 1, indicatorId: "preview", builtIn: false, version: 0, sourceHash: bodyHash(request.source), engine: this.runtime.engine, wrapperVersion: indicatorWrapperVersion,
      coin: request.coin, interval: request.interval, start: request.start, end: window.end, inputSchemaHash: bodyHash(canonical(meta.inputs)), computedAt: Date.now(),
      candleHash: bodyHash(canonical({ coin: request.coin, interval: request.interval, intervalMs: window.width, candles: window.candles })), resultHash: bodyHash(canonical(meta)),
      dataset: { count: window.candles.length, firstOpenTime: window.candles[0]!.openTime / 1000, lastOpenTime: window.candles.at(-1)!.openTime / 1000 },
      outputDataset: { count: meta.values.length, firstTime: meta.values[0]?.time ?? null, lastTime: meta.values.at(-1)?.time ?? null },
      coverage: { complete: true, closedOnly: true, expected: window.expected, returned: window.candles.length, seedCoverage: "complete" } };
    if (Buffer.byteLength(JSON.stringify(result)) > indicatorLimits.resultBytes) throw new IndicatorError("Full indicator result exceeds byte limit");
    return result;
  }
  save(agentId: string, request: IndicatorSaveRequest, operationKey: string, actor: IndicatorActor, options: { signal?: AbortSignal } = {}) {
    return this.work(agentId, options.signal, async (signal) => {
      const result = await this.store.save(agentId, request, operationKey, actor, async () => {
        if (!request.validation) throw new IndicatorError("Validation window is required");
        const validated = await this.previewSource(agentId, { ...request.validation, source: request.source }, signal);
        return { inputs: validated.inputs, overlay: validated.overlay };
      }, signal, this.runtime.engine.version);
      this.clear(agentId, result.indicatorId);
      for (const listener of this.listeners.get(agentId) ?? []) { try { listener(result); } catch (error) { this.reportError(error); } }
      return result;
    });
  }
  renderChart(agentId: string, layout: ChartLayout, end: number, options: { signal?: AbortSignal } = {}) {
    return this.work(agentId, options.signal, async (signal) => {
      const { coin, interval, indicators } = layout.document, width = intervalMillis[interval];
      if (!Number.isSafeInteger(end) || end % width !== 0 || end > Math.floor(Date.now() / width) * width) throw new IndicatorError("Invalid chart closed boundary");
      const start = Math.max(0, end - indicatorLimits.candles * width);
      const identity = { layoutId: layout.id, revision: layout.revision, coin, interval, end };
      const enabled = indicators.filter((instance) => !instance.hidden);
      if (!enabled.length) return { ...identity, instances: [] as { instanceId: string; result?: IndicatorResult; error?: string; code?: string }[] };
      const window = await fetchIndicatorWindow(this.market, coin, interval, start, end, signal, true);
      const instances: { instanceId: string; result?: IndicatorResult; error?: string; code?: string }[] = [];
      for (let index = 0; index < enabled.length; index += 2) {
        const pair = await Promise.all(enabled.slice(index, index + 2).map(async (instance) => {
          try {
            const current = await this.resolve(agentId, instance.indicatorId);
            if (current.descriptor.version !== instance.configuredVersion && Object.keys(instance.inputs).length) {
              const previous = await this.resolve(agentId, instance.indicatorId, instance.configuredVersion);
              validateInputMigration(previous.descriptor.inputs, current.descriptor.inputs, instance.inputs, true);
            }
            return { instanceId: instance.id, result: await this.calculate(agentId, current.descriptor, current.source, { coin, interval, start, end, inputs: instance.inputs, source: current.source }, signal, window) };
          } catch (error) {
            signal.throwIfAborted();
            return { instanceId: instance.id, error: error instanceof IndicatorError ? error.message : "Indicator calculation failed", code: error instanceof IndicatorError ? error.code : "indicator_runtime" };
          }
        }));
        instances.push(...pair);
      }
      return { ...identity, instances };
    });
  }
  async watch(agentId: string, signal: AbortSignal, publish: (event: { revision: number }) => void) {
    await this.admit(agentId);
    await new Promise<void>((resolve) => {
      let revision = Date.now();
      const off = this.subscribe(agentId, () => publish({ revision: ++revision }));
      const stop = () => { off(); signal.removeEventListener("abort", stop); resolve(); };
      signal.addEventListener("abort", stop, { once: true });
      if (signal.aborted) stop(); else publish({ revision });
    });
  }
  subscribe(agentId: string, listener: (event: CatalogEvent) => void): () => void {
    if (this.stopping || this.deleted.has(agentId)) throw new IndicatorError("Indicator service is unavailable", 503);
    let set = this.listeners.get(agentId);
    if (!set) { set = new Set(); this.listeners.set(agentId, set); }
    set.add(listener);
    return () => { set.delete(listener); if (!set.size) this.listeners.delete(agentId); };
  }
  async deleteAgent(agentId: string) {
    this.deleted.add(agentId);
    const jobs = [...this.jobs].filter((job) => job.agentId === agentId);
    for (const job of jobs) job.controller.abort(new Error("Agent deleted"));
    await this.runtime.abortAgent(agentId);
    await Promise.all(jobs.map((job) => job.done));
    this.clear(agentId); this.listeners.delete(agentId);
  }
  close(): Promise<void> {
    return this.closing ??= (async () => {
      this.stopping = true;
      for (const job of this.jobs) job.controller.abort(new Error("Indicator service closed"));
      await this.runtime.close();
      await Promise.all([...this.jobs].map((job) => job.done));
      await this.initializing?.catch(() => {});
      this.cache.clear(); this.cacheBytes = 0; this.listeners.clear(); this.deleted.clear();
    })();
  }
}
export function indicatorProjectionLimits(request: { count?: number; markerLimit?: number; saveAs?: string }) {
  const count = request.count ?? indicatorLimits.count, markerLimit = request.markerLimit ?? indicatorLimits.markerLimit;
  if (!Number.isSafeInteger(count) || count < 1 || count > indicatorLimits.maxCount || (count > indicatorLimits.inlineCount && !request.saveAs)) throw new IndicatorError("count must be 1–5,000; above 500 requires saveAs");
  if (!Number.isSafeInteger(markerLimit) || markerLimit < 0 || markerLimit > indicatorLimits.maxMarkerLimit) throw new IndicatorError("markerLimit must be 0–100");
  return { count, markerLimit };
}
export function projectIndicatorResult(result: IndicatorResult, request: { count?: number; markerLimit?: number; saveAs?: string } = {}) {
  const { count, markerLimit } = indicatorProjectionLimits(request);
  const { outputs, levels, fills, drawings, markers, values, ...provenance } = result;
  const projection = { ...provenance, outputs: outputs.map((output) => ({ id: output.id, title: output.title, kind: output.kind, overlay: output.overlay, forceOverlay: output.forceOverlay, style: output.style, finiteCount: output.finiteCount, firstFiniteTime: output.firstFiniteTime })), levels, fillCount: fills.length,
    values: values.slice(-count), numericalCoverage: { available: values.length, returned: Math.min(count, values.length) },
    markers: markerLimit === 0 ? [] : markers.slice(-markerLimit), markerCoverage: { available: markers.length, returned: Math.min(markerLimit, markers.length), firstSourceTime: markers[0]?.time ?? null, lastSourceTime: markers.at(-1)?.time ?? null },
    drawings: { count: drawings.length, byKind: { line: drawings.filter((d) => d.kind === "line").length, box: drawings.filter((d) => d.kind === "box").length, label: drawings.filter((d) => d.kind === "label").length } } };
  if (!request.saveAs && Buffer.byteLength(JSON.stringify(projection)) > indicatorLimits.inlineBytes) throw new IndicatorError("Inline indicator result exceeds 24 KiB; use a smaller count/markerLimit or saveAs");
  return projection;
}

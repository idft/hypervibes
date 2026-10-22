import { CodemodeSandbox } from "@earendil-works/pi-codemode";
import { readFile } from "node:fs/promises";
import { indicatorEngine, indicatorLimits, type IndicatorEngineInfo, type IndicatorLimits, type IndicatorVmRequest } from "./contracts.js";
import { normalizePineContext } from "./normalize.js";
import { runHyperVibesIndicator } from "./runtime-wrapper.js";
import { validateNormalizedIndicatorResult } from "./result-schema.js";
import { IndicatorError, validateInputs, validateSource } from "./validation.js";

/** Startup drift message; null when the installed pin matches the validated pin. */
export function pineVersionDrift(installed: string, expected: string): string | null {
  return installed === expected ? null : `pinets@${installed} is installed, but this HyperVibes build was validated against pinets@${expected}. Indicators run on the installed engine and record that version in results and source history. Update indicatorEngine in src/indicators/contracts.ts and re-run pnpm test:server src/indicators when upgrading.`;
}

/** One application-owned no-capability execution owner; capacity lasts until actual worker exit. */
export class IndicatorRuntime {
  readonly limits: IndicatorLimits;
  private bundle?: string;
  private initializing?: Promise<void>;
  private stopping = false;
  private closing?: Promise<void>;
  private workers = 0;
  private readonly perAgent = new Map<string, number>();
  private readonly exits = new Set<Promise<void>>();
  private readonly calls = new Set<{ agentId: string; controller: AbortController; done: Promise<void> }>();
  private readonly deleted = new Set<string>();
  private engineInfo: IndicatorEngineInfo = indicatorEngine;
  /** The engine version actually installed; recorded in results and source history. Available after initialize(). */
  get engine(): IndicatorEngineInfo { return this.engineInfo; }
  constructor(limits: Partial<IndicatorLimits> = {}, private readonly reportVersionDrift: (message: string) => void = () => {}) {
    this.limits = { ...indicatorLimits, ...limits };
    for (const key of Object.keys(this.limits) as (keyof IndicatorLimits)[]) {
      const value = this.limits[key];
      if (!Number.isSafeInteger(value) || value < 1 || value > indicatorLimits[key]) throw new Error(`Invalid indicator budget: ${key}`);
    }
  }
  initialize(): Promise<void> {
    if (this.stopping) return Promise.reject(new IndicatorError("Indicator runtime is closed", 503));
    return this.initializing ??= (async () => {
      const entry = new URL(import.meta.resolve("pinets"));
      const pkg = JSON.parse(await readFile(new URL("../package.json", entry), "utf8")) as { name?: unknown; version?: unknown };
      // Structural failures stay fatal: the wrong/missing package or an unreadable
      // version makes every result's provenance a lie. A mere version drift does
      // not: execution continues on the installed engine and records its actual
      // version, so a routine dependency bump can never brick startup.
      if (pkg.name !== "pinets" || typeof pkg.version !== "string" || !/^\d+\.\d+\.\d+(-[\w.+-]+)?$/.test(pkg.version)) throw new Error("The installed pinets package is missing or unreadable; indicator execution is unavailable");
      this.engineInfo = { name: "pinets", version: pkg.version };
      if (pkg.version !== indicatorEngine.version) { const drift = pineVersionDrift(pkg.version, indicatorEngine.version); if (drift) this.reportVersionDrift(drift); }
      const bundle = await readFile(new URL("pinets.min.browser.js", entry), "utf8");
      if (!this.stopping) this.bundle = bundle;
    })();
  }
  private admit(agentId: string) {
    if (this.stopping || this.deleted.has(agentId)) throw new IndicatorError("Indicator execution is shutting down", 503, "indicator_closed");
    const admitted = [...this.calls].filter((call) => call.agentId === agentId).length;
    if (this.workers >= this.limits.workers || (this.perAgent.get(agentId) ?? 0) >= this.limits.workersPerAgent || this.calls.size >= this.limits.workers || admitted >= this.limits.workersPerAgent) throw new IndicatorError("Indicator workers are busy; retry later", 429, "indicator_busy");
  }
  private reserve(agentId: string) {
    if (this.stopping || this.deleted.has(agentId)) throw new IndicatorError("Indicator runtime is closed", 503);
    if (this.workers >= this.limits.workers || (this.perAgent.get(agentId) ?? 0) >= this.limits.workersPerAgent) throw new IndicatorError("Indicator workers are busy; retry later", 429);
    this.workers++;
    this.perAgent.set(agentId, (this.perAgent.get(agentId) ?? 0) + 1);
    return () => {
      this.workers--;
      const remaining = (this.perAgent.get(agentId) ?? 1) - 1;
      if (remaining) this.perAgent.set(agentId, remaining); else this.perAgent.delete(agentId);
    };
  }
  private prepare(request: Omit<IndicatorVmRequest, "limits">): IndicatorVmRequest {
    validateSource(request.source, this.limits.sourceBytes);
    const inputs = validateInputs(request.inputs);
    if (!request.candles.length || request.candles.length > this.limits.candles) throw new IndicatorError("Invalid indicator candle count");
    if (!Number.isSafeInteger(request.intervalMs) || request.intervalMs <= 0) throw new IndicatorError("Invalid indicator interval");
    for (const [index, candle] of request.candles.entries()) {
      if (!Object.values(candle).every((value) => typeof value === "number" && Number.isFinite(value)) || !Number.isSafeInteger(candle.openTime) || candle.closeTime !== candle.openTime + request.intervalMs
        || candle.volume < 0 || candle.low > Math.min(candle.open, candle.close) || candle.high < Math.max(candle.open, candle.close)
        || (index > 0 && request.candles[index - 1]!.closeTime !== candle.openTime)) throw new IndicatorError("Invalid or gapped indicator candles");
    }
    return { ...request, inputs, limits: this.limits };
  }
  async execute(agentId: string, request: Omit<IndicatorVmRequest, "limits">, signal?: AbortSignal) {
    signal?.throwIfAborted();
    this.admit(agentId);
    if (!this.bundle) throw new IndicatorError("Indicator runtime is not initialized", 503);
    const payload = this.prepare(request);
    const controller = new AbortController();
    let settled!: () => void;
    const call = { agentId, controller, done: new Promise<void>((resolve) => { settled = resolve; }) };
    this.calls.add(call);
    // The host starts this service before it has a listening socket. WASM loading
    // and worker termination can otherwise leave only unreferenced promises in Node.
    const keepAlive = setInterval(() => { /* Keep admitted execution/exit observable. */ }, 1000);
    let started = false;
    const sandbox = new CodemodeSandbox({ tools: [], globals: [], timeoutMs: this.limits.timeoutMs, memoryLimitBytes: this.limits.memoryLimitBytes,
      acquireWorker: () => { const release = this.reserve(agentId); return () => { release(); clearInterval(keepAlive); }; },
      onWorkerStarted: (exit) => { started = true; this.exits.add(exit); void exit.then(() => this.exits.delete(exit)); },
    });
    try {
      // tsx emits esbuild's __name helper inside function bodies. Bind that trusted
      // compiler helper in the VM too; never capture a Node closure in serialized code.
      const code = `${this.bundle}\nreturn await ((__name) => (${runHyperVibesIndicator.toString()})(${JSON.stringify(payload)}, PineTSLib, (${normalizePineContext.toString()})))((target, name) => Object.defineProperty(target, "name", { value: name, configurable: true }));`;
      const result = await sandbox.execute(code, { signal: signal ? AbortSignal.any([signal, controller.signal]) : controller.signal, timeoutMs: this.limits.timeoutMs });
      if (!result.ok) throw new IndicatorError(`PineTS ${result.error.kind}: ${result.error.message}`, result.error.kind === "aborted" ? 499 : 400, `indicator_${result.error.kind}`);
      if (result.calls.length) throw new IndicatorError("Indicator attempted a host tool call");
      // storeWrites are deliberately discarded; indicator calculations have no persistent capabilities.
      return validateNormalizedIndicatorResult(result.value, this.limits);
    } finally {
      try { await sandbox.close(); }
      finally { if (!started) clearInterval(keepAlive); this.calls.delete(call); settled(); }
    }
  }
  /** One worker for several trusted calculations (startup built-in validation): a
   * single bundle parse and VM spawn instead of one per program. */
  async executeBatch(agentId: string, requests: Omit<IndicatorVmRequest, "limits">[], signal?: AbortSignal) {
    signal?.throwIfAborted();
    if (!requests.length || requests.length > 16) throw new IndicatorError("Invalid indicator batch");
    this.admit(agentId);
    if (!this.bundle) throw new IndicatorError("Indicator runtime is not initialized", 503);
    const payloads = requests.map((request) => this.prepare(request));
    const controller = new AbortController();
    let settled!: () => void;
    const call = { agentId, controller, done: new Promise<void>((resolve) => { settled = resolve; }) };
    this.calls.add(call);
    const keepAlive = setInterval(() => { /* Keep admitted execution/exit observable. */ }, 1000);
    let started = false;
    const sandbox = new CodemodeSandbox({ tools: [], globals: [], timeoutMs: this.limits.timeoutMs, memoryLimitBytes: this.limits.memoryLimitBytes,
      acquireWorker: () => { const release = this.reserve(agentId); return () => { release(); clearInterval(keepAlive); }; },
      onWorkerStarted: (exit) => { started = true; this.exits.add(exit); void exit.then(() => this.exits.delete(exit)); },
    });
    try {
      const code = `${this.bundle}\nreturn await ((__name) => (async () => { const results = []; for (const request of ${JSON.stringify(payloads)}) results.push(await (${runHyperVibesIndicator.toString()})(request, PineTSLib, (${normalizePineContext.toString()}))); return results; })())((target, name) => Object.defineProperty(target, "name", { value: name, configurable: true }));`;
      const result = await sandbox.execute(code, { signal: signal ? AbortSignal.any([signal, controller.signal]) : controller.signal, timeoutMs: this.limits.timeoutMs });
      if (!result.ok) throw new IndicatorError(`PineTS ${result.error.kind}: ${result.error.message}`, result.error.kind === "aborted" ? 499 : 400, `indicator_${result.error.kind}`);
      if (result.calls.length) throw new IndicatorError("Indicator attempted a host tool call");
      const values = result.value;
      if (!Array.isArray(values) || values.length !== payloads.length) throw new IndicatorError("Malformed indicator batch result");
      return values.map((value) => validateNormalizedIndicatorResult(value, this.limits));
    } finally {
      try { await sandbox.close(); }
      finally { if (!started) clearInterval(keepAlive); this.calls.delete(call); settled(); }
    }
  }
  async abortAgent(agentId: string) {
    this.deleted.add(agentId);
    const active = [...this.calls].filter((call) => call.agentId === agentId);
    for (const call of active) call.controller.abort(new Error("Agent deleted"));
    await Promise.all(active.map((call) => call.done));
    // Exits may include other Agents; deletion must not wait for their work.
    while ((this.perAgent.get(agentId) ?? 0) > 0) await Promise.race(this.exits);
  }
  close(): Promise<void> {
    return this.closing ??= (async () => {
      this.stopping = true;
      for (const call of this.calls) call.controller.abort(new Error("Indicator runtime closed"));
      await Promise.all([...this.calls].map((call) => call.done));
      await Promise.allSettled([...this.exits]);
      await this.initializing?.catch(() => {});
      this.bundle = undefined;
      this.deleted.clear();
    })();
  }
}

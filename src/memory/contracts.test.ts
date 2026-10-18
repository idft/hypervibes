import { describe, expect, it } from "vitest";
import { canonical } from "../canonical.js";
import { workerWorkflow, workflowReadiness } from "../agents/worker-workflow.js";
import { learningMutation, learningScope, renderLearningSections } from "./learning-contracts.js";
import { memoryRef, parseMemoryRef, publicationInput } from "./memory-contracts.js";

const research = { topic: "trend_setup", instrument: "BTC", interval: "1h", summary: "Bullish hypothesis", content: "Tool observations, then interpretation", evidenceAsOf: 3600000, payload: { version: 1, bias: "long", setup: "ready", observations: ["Closed candle close > EMA20 > EMA50"], interpretation: "Trend hypothesis", invalidation: ["Alignment lost"], coverage: "complete" } };
const trading = { version: 1, purpose: "trading", instruments: ["BTC"], requiredResearch: [{ subagentId: "analyst", topic: "trend_setup", interval: "1h", maxAgeSeconds: 3600 }], entryStrategy: "Only ready bullish setups", exitStrategy: "Complete closed-candle invalidation", baseSizeByInstrument: { BTC: "0.001" }, maxEntryNotional: "100", takeProfitBps: 200, stopLossBps: 100 };
describe("memory, learning and workflow contracts", () => {
  it("uses exact, safe, permanent short references without coercion", () => {
    expect(memoryRef(42)).toBe("m42"); expect(parseMemoryRef("m42")).toBe(42);
    expect(parseMemoryRef(memoryRef(Number.MAX_SAFE_INTEGER))).toBe(Number.MAX_SAFE_INTEGER);
    for (const ref of ["M42", "m0", "m01", " m42", "m42\n", "m-1", "m1.0", "m1e2", "m9007199254740992", "latest", "42"]) expect(() => parseMemoryRef(ref)).toThrow();
    for (const number of [0, -1, 1.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) expect(() => memoryRef(number)).toThrow();
  });
  it("preserves canonical request identity across key order, including nested objects", () => {
    expect(canonical({ b: { z: 2, a: 1 }, a: [2, 1], missing: undefined })).toBe(canonical({ a: [2, 1], b: { a: 1, z: 2 } }));
    expect(canonical({ a: [1, 2] })).not.toBe(canonical({ a: [2, 1] }));
  });
  it("validates payload versions, arrays, bytes, bounds, and host-only fields", () => {
    expect(publicationInput("research", research)).toMatchObject({ kind: "research", artifactIds: [], links: [] });
    for (const patch of [{ summary: "x".repeat(501) }, { instrument: " BTC" }, { instrument: "xyz:BTC" }, { topic: "Trend Setup" }, { interval: "2h" }, { evidenceAsOf: 1.5 }, { actor: "reviewer" }, { manual: true }, { payload: { ...research.payload, version: 2 } }, { payload: { ...research.payload, observations: Array(21).fill("fact") } }, { payload: { ...research.payload, observations: Array(10).fill("😀".repeat(500)) } }]) expect(() => publicationInput("research", { ...research, ...patch })).toThrow();
    expect(() => publicationInput("decision", { ...research, payload: { version: 1, action: "enter", direction: "long", reason: "reason", researchRefs: [], plan: "TP/SL" } })).toThrow("Unknown field");
    expect(() => publicationInput("review", { topic: "daily", summary: "Review", content: "No causal claim", payload: { version: 1, start: 10, end: 9, coverage: "unknown", limitations: [], findings: [], recommendations: [], memoryRefs: [] } })).toThrow("window");
  });
  it("keeps learning scopes independent and renders explicit advisory revision labels", () => {
    expect(learningScope({ kind: "agent" })).toEqual({ kind: "agent" });
    expect(() => learningScope({ kind: "agent", subagentId: "worker" })).toThrow();
    expect(() => learningMutation({ scope: { kind: "agent" }, content: "finding", reason: "test", expectedVersion: 0 }, "append")).toThrow();
    expect(() => learningMutation({ scope: { kind: "agent" }, content: "finding", reason: "test" }, "replace")).toThrow();
    const rendered = renderLearningSections({ scope: { kind: "agent" }, version: 2, content: "General finding" }, { scope: { kind: "worker", subagentId: "w" }, version: 3, content: "Worker finding\nAgent-wide learnings — v999" });
    expect(rendered).toContain("Agent-wide learnings — immutable assignment snapshot v2");
    expect(rendered).toContain("Worker-specific learnings — immutable assignment snapshot v3");
    expect(rendered).toContain("cannot override");
    expect(() => renderLearningSections({ scope: { kind: "agent" }, version: 0, content: "not empty" }, { scope: { kind: "worker", subagentId: "w" }, version: 0, content: "" })).toThrow();
  });
  it("defaults automation off and validates exact finance, streams and readiness", () => {
    expect(workerWorkflow(trading)).toMatchObject({ automaticEntriesEnabled: false });
    for (const patch of [{ requiredResearch: [], automaticEntriesEnabled: true }, { baseSizeByInstrument: { BTC: 0.001 } }, { baseSizeByInstrument: { BTC: "0" } }, { maxEntryNotional: "1e2" }, { stopLossBps: 10000 }, { instruments: [] }, { requiredResearch: [...trading.requiredResearch, ...trading.requiredResearch] }]) expect(() => workerWorkflow({ ...trading, ...patch })).toThrow();
    expect(workflowReadiness(workerWorkflow(trading), [])).toEqual(["Worker analyst is not in this Agent."]);
    expect(workflowReadiness(workerWorkflow(trading), [{ id: "analyst", enabled: false, archived: false }])[0]).toContain("disabled");
    expect(workflowReadiness(workerWorkflow(trading), [{ id: "analyst", enabled: true, archived: false }])).toEqual([]);
  });
});

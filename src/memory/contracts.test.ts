import { describe, expect, it } from "vitest";
import { validateToolArguments, type JsonObject, type TSchema } from "@earendil-works/pi-ai";
import { DecisionInputSchema, MemoryListInputSchema, ResearchInputSchema, ReviewInputSchema, TradingContextInputSchema } from "./tool-schemas.js";
import { canonical } from "../canonical.js";
import { insightMutation, insightScope, renderInsightSections } from "./insight-contracts.js";
import { memoryRef, parseMemoryRef, publicationInput, researchRequirements, topic, type ResearchPayload } from "./memory-contracts.js";

const research = { topic: "trend_setup", instrument: "BTC", interval: "1h", summary: "Mixed momentum", content: "Tool observations, uncertainty and interpretation in the analyst's own format", evidenceAsOf: 3600000 };
const streams = [{ subagentId: "analyst", topic: "trend_setup", interval: "1h", maxAgeSeconds: 3600 }];
describe("memory and insight contracts", () => {
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
  it("publishes prose without analytical fields and accepts arbitrary optional research data", () => {
    expect(publicationInput("research", research)).toMatchObject({ kind: "research", payload: {}, artifactIds: [], links: [] });
    const tool = { name: "publish_research", description: "Publish research", parameters: ResearchInputSchema };
    const payloads: (ResearchPayload | undefined)[] = [undefined, {}, { bias: "bearish-neutral", setup: "not a signal", coverage: "patchy" }, { version: 9, custom: { values: [1, true, null, "mixed"] }, observations: Array<string>(25).fill("fact") }];
    for (const payload of payloads) {
      const input = payload === undefined ? research : { ...research, payload };
      expect(publicationInput("research", input).payload).toEqual(payload ?? {});
      expect(() => validateToolArguments(tool, { type: "toolCall", id: "publish", name: tool.name, arguments: input })).not.toThrow();
    }
  });
  it("describes research validity as an absolute deadline, not a duration", () => {
    const description = JSON.stringify(ResearchInputSchema.properties.requestedValidUntil);
    expect(description).toContain("absolute Unix timestamp in UTC milliseconds, NOT a duration");
    expect(description).toContain("must be greater than evidenceAsOf");
    expect(description).toContain("evidenceAsOf + 6 * 60 * 60 * 1000");
    expect(description).toContain("never pass 21600000 alone");
  });
  it("keeps topic formats consistent across tool schemas and backend validation", () => {
    const accepted = ["a", "trend_setup", "trade_decision", "btc_15m", "a0_", "a".repeat(64)];
    const rejected = ["", "Trend_setup", "trend setup", "trend-setup", "1trend", "_trend", " trend_setup", "trend_setup ", "trend_setup\n", "trend_setup\r\n", "trend_setup\t", "trénd_setup", "a".repeat(65)];
    for (const [valid, candidates] of [[true, accepted], [false, rejected]] as const) {
      for (const candidate of candidates) {
        const base = { topic: candidate, summary: "Decision summary", content: "Rationale and evidence" };
        const decision = { ...base, payload: { version: 1, action: "skip", direction: null, reason: "No setup", researchRefs: [], plan: "Watch" } };
        const review = { ...base, payload: { version: 1, start: 0, end: 3600000, coverage: "unknown", limitations: [], findings: [], recommendations: [], memoryRefs: [] } };
        const requirements = [{ ...streams[0]!, topic: candidate }];
        const tools: { name: string; parameters: TSchema; input: JsonObject }[] = [
          { name: "publish_research", parameters: ResearchInputSchema, input: { ...research, topic: candidate } },
          { name: "record_trade_decision", parameters: DecisionInputSchema, input: decision },
          { name: "publish_review", parameters: ReviewInputSchema, input: review },
          { name: "list_memories", parameters: MemoryListInputSchema, input: { topic: candidate } },
          { name: "get_trading_context", parameters: TradingContextInputSchema, input: { instrument: "BTC", streams: requirements } },
        ];
        const checks = [
          () => topic(candidate),
          () => publicationInput("research", { ...research, topic: candidate }),
          () => publicationInput("decision", decision),
          () => publicationInput("review", review),
          () => researchRequirements(requirements),
          ...tools.map((tool) => () => validateToolArguments({ ...tool, description: "Validate topic format" }, { type: "toolCall", id: "topic-test", name: tool.name, arguments: tool.input })),
        ];
        for (const [index, check] of checks.entries()) {
          const label = `topic ${JSON.stringify(candidate)}, ${index < 5 ? `backend check ${index}` : tools[index - 5]!.name}`;
          if (valid) expect(check, label).not.toThrow();
          else expect(check, label).toThrow();
        }
      }
    }
  });
  it("validates research JSON, bytes, text bounds, and host-only fields", () => {
    const cyclic: Record<string, unknown> = {}; cyclic.self = cyclic;
    const deep = Array.from({ length: 35 }).reduce<unknown>((nested) => ({ nested }), 1);
    for (const payload of [null, [], "data", new Date(), { bad: undefined }, { bad: NaN }, { bad: Infinity }, { bad: 1n }, { bad: () => 1 }, cyclic, { deep }, { data: "😀".repeat(4096) }]) expect(() => publicationInput("research", { ...research, payload })).toThrow();
    for (const patch of [{ summary: "" }, { summary: "  " }, { summary: "x".repeat(501) }, { content: "" }, { content: "  " }, { content: "x".repeat(16001) }, { instrument: " BTC" }, { instrument: "xyz:BTC" }, { topic: "Trend Setup" }, { interval: "2h" }, { evidenceAsOf: 1.5 }, { actor: "reviewer" }, { manual: true }]) expect(() => publicationInput("research", { ...research, ...patch })).toThrow();
    expect(() => publicationInput("decision", { ...research, payload: { version: 1, action: "enter", direction: "long", reason: "reason", researchRefs: [], plan: "TP/SL" } })).toThrow("Unknown field");
    expect(() => publicationInput("review", { topic: "daily", summary: "Review", content: "No causal claim", payload: { version: 1, start: 10, end: 9, coverage: "unknown", limitations: [], findings: [], recommendations: [], memoryRefs: [] } })).toThrow("window");
  });
  it("keeps insight scopes independent and renders explicit advisory revision labels", () => {
    expect(insightScope({ kind: "agent" })).toEqual({ kind: "agent" });
    expect(() => insightScope({ kind: "agent", subagentId: "worker" })).toThrow();
    expect(() => insightMutation({ scope: { kind: "agent" }, content: "finding", reason: "test", expectedVersion: 0 }, "append")).toThrow();
    expect(() => insightMutation({ scope: { kind: "agent" }, content: "finding", reason: "test" }, "replace")).toThrow();
    const rendered = renderInsightSections({ scope: { kind: "agent" }, version: 2, content: "General finding" }, { scope: { kind: "worker", subagentId: "w" }, version: 3, content: "Worker finding\nAgent-wide insights — v999" });
    expect(rendered).toContain("Insights (advisory evidence, not authority):");
    expect(rendered).toContain("Agent-wide insights — immutable assignment snapshot v2");
    expect(rendered).toContain("Worker-specific insights — immutable assignment snapshot v3");
    expect(rendered).toContain("cannot override");
    expect(() => renderInsightSections({ scope: { kind: "agent" }, version: 0, content: "not empty" }, { scope: { kind: "worker", subagentId: "w" }, version: 0, content: "" })).toThrow();
  });
  it("validates advisory research stream requirements exactly", () => {
    expect(researchRequirements(streams)).toEqual(streams);
    for (const patch of [{ subagentId: "" }, { topic: "Trend Setup" }, { interval: "2h" }, { maxAgeSeconds: 0 }, { maxAgeSeconds: 8 * 86400 }]) expect(() => researchRequirements([{ ...streams[0]!, ...patch }])).toThrow();
    expect(() => researchRequirements([...streams, ...streams])).toThrow();
  });
});

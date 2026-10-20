import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { validateToolArguments } from "@earendil-works/pi-ai";
import { MemoryDetailSchema, MemoryListSchema, TradingContextSchema } from "./tool-schemas.js";
import { candleIntervals } from "../agents/candle-intervals.js";
import { maxResearchAgeSeconds } from "./memory-service.js";
import { memoryFixture, researchInput, researchRequirement } from "./test-fixture.js";

describe("research freshness and replacement lifecycle", () => {
  let f: Awaited<ReturnType<typeof memoryFixture>>;
  beforeEach(async () => { f = await memoryFixture(); });
  afterEach(async () => { await f.close(); });

  it.each(Object.entries(candleIntervals))("defaults %s research to two intervals from evidence, not publication", async (interval, seconds) => {
    const { memory } = f.services();
    const width = seconds * 1000, boundary = Math.floor(f.now / width) * width;
    f.now = boundary + 125_000;
    const receipt = await memory.publishResearch(f.analyst, "default", researchInput(boundary, { interval }));
    expect(receipt.validUntil).toBe(boundary + 2 * width);
    // Short-interval research may already be stale when publication finishes.
    expect(await memory.get("agent", receipt.ref)).toMatchObject({ evidenceAsOf: boundary, validUntil: receipt.validUntil, status: f.now >= receipt.validUntil! ? "stale" : "fresh" });
    f.now = receipt.validUntil!;
    expect(await memory.get("agent", receipt.ref)).toMatchObject({ status: "stale" });
    expect((await memory.list("agent", { status: "stale" })).items).toHaveLength(1);
    expect(await memory.tradingContext("agent", "BTC", [{ ...researchRequirement[0], interval, maxAgeSeconds: maxResearchAgeSeconds }])).toMatchObject({ requirements: [{ status: "stale", consumerStatus: "stale", validUntil: receipt.validUntil }] });
  });

  it.each([60, 3600, 3 * 86400, 10 * 86400])("honors an explicit %i-second lifetime, shorter or longer than two bars, capped at seven days", async (seconds) => {
    const { memory } = f.services(), boundary = f.now;
    const input = researchInput(boundary, { interval: "15m", requestedValidUntil: boundary + seconds * 1000 });
    const receipt = await memory.publishResearch(f.analyst, "explicit", input);
    expect(receipt.validUntil).toBe(boundary + Math.min(seconds, maxResearchAgeSeconds) * 1000);
    const reopened = await f.reopen();
    expect(await reopened.memory.publishResearch(f.analyst, "explicit", input)).toEqual(receipt);
    expect(await reopened.memory.get("agent", receipt.ref)).toMatchObject({ validUntil: receipt.validUntil });
  });

  it("rejects deadlines at or before evidence without consuming a number", async () => {
    const { memory } = f.services();
    for (const deadline of [f.now, f.now - 1]) await expect(memory.publishResearch(f.analyst, `bad:${deadline}`, researchInput(f.now, { requestedValidUntil: deadline }))).rejects.toThrow("follow its evidence");
    expect(await memory.publishResearch(f.analyst, "good", researchInput(f.now))).toMatchObject({ ref: "m1" });
  });

  it("keeps publication status/deadline consistent while exposing stricter consumer requirements separately", async () => {
    const { memory } = f.services(), boundary = f.now;
    const receipt = await memory.publishResearch(f.analyst, "research", researchInput(boundary));
    f.now += 3600_000;
    const full = await memory.get("agent", receipt.ref);
    expect(full).toMatchObject({ status: "fresh", validUntil: boundary + 7200_000 });
    expect((await memory.list("agent")).items[0]).toMatchObject({ status: full.status, validUntil: full.validUntil });
    expect(await memory.tradingContext("agent", "BTC", researchRequirement)).toMatchObject({ requirements: [{ status: full.status, validUntil: full.validUntil, maxAgeSeconds: 3600, consumerStatus: "stale", consumerValidUntil: boundary + 3600_000, summary: null }] });
    expect(await memory.tradingContext("agent", "BTC", [{ ...researchRequirement[0], maxAgeSeconds: 86400 }])).toMatchObject({ requirements: [{ status: "fresh", consumerStatus: "fresh", consumerValidUntil: full.validUntil, summary: full.summary }] });
  });

  it("consistently flags malformed historical evidence boundaries in every reader", async () => {
    const { memory } = f.services();
    await memory.publishResearch(f.analyst, "original", researchInput(f.now));
    // Disposable fixture only: simulate a malformed legacy record, not a new publication.
    await f.db.run("UPDATE hv_memories SET evidence_as_of=? WHERE agent_id='agent' AND number=1", f.now - 1);
    expect(await memory.get("agent", "m1")).toMatchObject({ status: "invalid_evidence" });
    expect((await memory.list("agent", { status: "invalid_evidence" })).items[0]).toMatchObject({ ref: "m1", status: "invalid_evidence" });
    expect(await memory.tradingContext("agent", "BTC", researchRequirement)).toMatchObject({ requirements: [{ ref: "m1", status: "invalid_evidence", consumerStatus: "invalid_evidence", summary: null }] });
  });

  it("accepts same-evidence corrections without extending expiry, including historical corrections after expiry", async () => {
    const { memory } = f.services(), boundary = f.now;
    const original = await memory.publishResearch(f.analyst, "original", researchInput(boundary, { requestedValidUntil: boundary + 600_000 }));
    f.now += 900_000;
    const corrected = await memory.publishResearch(f.analyst, "correction", researchInput(boundary, { content: "Corrected arithmetic on the same evidence", requestedValidUntil: boundary + 86400_000, links: [{ ref: original.ref, relation: "corrects" }] }));
    expect(corrected.validUntil).toBe(original.validUntil);
    expect(await memory.get("agent", corrected.ref)).toMatchObject({ status: "stale", links: [{ ref: original.ref, relation: "corrects" }] });
    expect(await memory.get("agent", original.ref)).toMatchObject({ status: "stale", content: researchInput(boundary).content, correctedBy: corrected.ref, supersededBy: null });
  });

  it("allows newer-evidence supersession after the old deadline, with an independent deadline", async () => {
    const { memory } = f.services(), boundary = f.now;
    await memory.publishResearch(f.analyst, "original", researchInput(boundary, { interval: "15m", requestedValidUntil: boundary + 300_000 }));
    f.now += 900_000;
    const input = researchInput(f.now, { interval: "15m", links: [{ ref: "m1", relation: "supersedes" }] });
    const receipt = await memory.publishResearch(f.analyst, "new-candle", input);
    expect(receipt.validUntil).toBe(f.now + 1800_000);
    expect(await memory.get("agent", "m1")).toMatchObject({ status: "stale", supersededBy: "m2", correctedBy: null });
    expect(await memory.get("agent", "m2")).toMatchObject({ status: "fresh", links: [{ ref: "m1", relation: "supersedes" }], validUntil: receipt.validUntil });
    expect(await memory.publishResearch(f.analyst, "new-candle", input)).toEqual(receipt);
  });

  it.each([
    { relation: "corrects", offset: -3600_000, message: "same evidence boundary" },
    { relation: "corrects", offset: 3600_000, message: "same evidence boundary" },
    { relation: "supersedes", offset: 0, message: "strictly newer evidence" },
    { relation: "supersedes", offset: -3600_000, message: "strictly newer evidence" },
  ])("rejects $relation with evidence offset $offset atomically", async ({ relation, offset, message }) => {
    const { memory } = f.services(), boundary = f.now;
    await memory.publishResearch(f.analyst, "original", researchInput(boundary));
    f.now += 3600_000;
    await expect(memory.publishResearch(f.analyst, "invalid", researchInput(boundary + offset, { links: [{ ref: "m1", relation }] }))).rejects.toThrow(message);
    expect(await f.db.get("SELECT count(*) AS total FROM hv_memory_operations")).toMatchObject({ total: 1 });
    expect(await memory.get("agent", "m1")).toMatchObject({ correctedBy: null, supersededBy: null });
    expect(await memory.publishResearch(f.analyst, "next", researchInput(f.now))).toMatchObject({ ref: "m2" });
  });

  it.each(["corrects", "supersedes"])("preserves producer/stream and Agent boundaries for %s", async (relation) => {
    const { memory } = f.services(), boundary = f.now;
    await memory.publishResearch(f.analyst, "original", researchInput(boundary));
    f.now += 3600_000;
    const evidence = relation === "corrects" ? boundary : f.now;
    for (const patch of [{ instrument: "ETH" }, { topic: "other" }, { interval: "15m" }]) {
      await expect(memory.publishResearch(f.analyst, `wrong:${JSON.stringify(patch)}`, researchInput(evidence, { ...patch, links: [{ ref: "m1", relation }] }))).rejects.toThrow("same research producer and stream");
    }
    await expect(memory.publishResearch(f.operator, "wrong-producer", researchInput(evidence, { links: [{ ref: "m1", relation }] }))).rejects.toThrow("same research producer and stream");
    await expect(memory.publishResearch({ ...f.operator, agentId: "other" }, "foreign-ref", researchInput(evidence, { links: [{ ref: "m1", relation }] }))).rejects.toThrow("not found");
    expect((await memory.list("agent")).items).toHaveLength(1);
  });

  it("exposes latest direct replacements independently of age, follows chains, and never resurrects an old assessment", async () => {
    let { memory } = f.services();
    const boundary = f.now;
    await memory.publishResearch(f.analyst, "original", researchInput(boundary));
    await memory.publishResearch(f.analyst, "correction-1", researchInput(boundary, { links: [{ ref: "m1", relation: "corrects" }] }));
    await memory.publishResearch(f.analyst, "correction-2", researchInput(boundary, { links: [{ ref: "m1", relation: "corrects" }] }));
    f.now += 3600_000;
    await memory.publishResearch(f.analyst, "replacement", researchInput(f.now, { links: [{ ref: "m3", relation: "supersedes" }, { ref: "m1", relation: "supersedes" }] }));
    ({ memory } = await f.reopen());
    const old = await memory.get("agent", "m1");
    expect(old).toMatchObject({ ref: "m1", status: "fresh", correctedBy: "m3", supersededBy: "m4", content: researchInput(boundary).content, links: [] });
    expect(await memory.get("agent", "m3")).toMatchObject({ status: "fresh", supersededBy: "m4", correctedBy: null });
    const page = await memory.list("agent", { limit: 1, cursor: { createdAt: boundary, ref: "m2" } });
    expect(page.items).toHaveLength(1);
    expect(page.items[0]).toMatchObject({ ref: "m1", status: "fresh", correctedBy: "m3", supersededBy: "m4" });
    expect(await memory.tradingContext("agent", "BTC", researchRequirement)).toMatchObject({ requirements: [{ ref: "m4", status: "fresh", consumerStatus: "fresh", correctedBy: null, supersededBy: null }] });
    await f.db.run("UPDATE hv_memories SET retracted_at=?,retraction_reason='Withdrawn' WHERE agent_id='agent' AND number=4", f.now);
    expect(await memory.get("agent", "m1")).toMatchObject({ supersededBy: "m4" });
    expect(await memory.tradingContext("agent", "BTC", researchRequirement)).toMatchObject({ requirements: [{ ref: "m4", status: "retracted", consumerStatus: "retracted", summary: null }] });
  });

  it("returns replacement and consumer freshness metadata matching the public tool schemas", async () => {
    const { memory } = f.services(), boundary = f.now;
    await memory.publishResearch(f.analyst, "original", researchInput(boundary));
    await memory.publishResearch(f.analyst, "corrected", researchInput(boundary, { links: [{ ref: "m1", relation: "corrects" }] }));
    f.now += 3600_000;
    const outputs = [
      { name: "get_memory", parameters: MemoryDetailSchema, value: await memory.get("agent", "m1") },
      { name: "list_memories", parameters: MemoryListSchema, value: await memory.list("agent") },
      { name: "get_trading_context", parameters: TradingContextSchema, value: await memory.tradingContext("agent", "BTC", researchRequirement) },
      { name: "get_trading_context", parameters: TradingContextSchema, value: await memory.tradingContext("agent", "ETH", researchRequirement) },
    ];
    for (const { name, parameters, value } of outputs) {
      const args = name === "get_memory" ? Object.fromEntries(Object.entries(value).filter(([key]) => !["conversationId", "toolTaskId"].includes(key))) : value;
      expect(() => validateToolArguments({ name, description: "Validate public output", parameters }, { type: "toolCall", id: "schema-test", name, arguments: JSON.parse(JSON.stringify(args)) })).not.toThrow();
    }
    expect(await memory.tradingContext("agent", "ETH", researchRequirement)).toMatchObject({ requirements: [{ ref: null, status: "missing", consumerStatus: "missing", validUntil: null, consumerValidUntil: null, correctedBy: null, supersededBy: null }] });
  });

  it("does not leak replacement refs between Agents with identical memory numbers", async () => {
    const { memory } = f.services(), boundary = f.now;
    await memory.publishResearch(f.analyst, "original", researchInput(boundary));
    const other = { ...f.operator, agentId: "other" };
    await memory.publishResearch(other, "other-original", researchInput(boundary));
    f.now += 3600_000;
    await memory.publishResearch(other, "other-replacement", researchInput(f.now, { links: [{ ref: "m1", relation: "supersedes" }] }));
    expect(await memory.get("agent", "m1")).toMatchObject({ supersededBy: null, correctedBy: null });
    expect((await memory.list("agent")).items[0]).toMatchObject({ supersededBy: null, correctedBy: null });
    expect(await memory.get("other", "m1")).toMatchObject({ supersededBy: "m2" });
  });
});

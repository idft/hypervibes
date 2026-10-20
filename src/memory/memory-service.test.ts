import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ArtifactService } from "../research/artifacts.js";
import { memoryFixture, researchInput, researchRequirement } from "./test-fixture.js";

describe("immutable Agent-scoped memories", () => {
  let f: Awaited<ReturnType<typeof memoryFixture>>;
  beforeEach(async () => { f = await memoryFixture(); });
  afterEach(async () => { await f.close(); });
  it("allocates transactionally, returns exact receipts after reopen and refuses identity reuse", async () => {
    let { memory } = f.services();
    const input = researchInput(f.now);
    const results = await Promise.all(Array.from({ length: 12 }, (_, n) => memory.publishResearch(f.analyst, `publish:${n}`, input)));
    expect(results.map((r) => r.ref)).toEqual(Array.from({ length: 12 }, (_, n) => `m${n + 1}`));
    const retries = await Promise.all([memory.publishResearch(f.analyst, "same", input), memory.publishResearch(f.analyst, "same", input)]);
    expect(retries[0]).toEqual(retries[1]);
    ({ memory } = await f.reopen());
    expect(await memory.publishResearch(f.analyst, "publish:0", input)).toEqual(results[0]);
    await expect(memory.publishResearch(f.analyst, "publish:0", { ...input, summary: "different" })).rejects.toThrow("different");
    await expect(memory.publishResearch({ ...f.analyst, toolTaskId: 3 }, "publish:0", input)).rejects.toThrow("actor");
    await expect(memory.get("other", "m1")).rejects.toThrow("not found");
    await expect(memory.get("agent", "m01")).rejects.toThrow("exact");
    expect((await memory.list("agent", { limit: 5 })).items).toHaveLength(5);
    const first = await memory.list("agent", { limit: 5 });
    const second = await memory.list("agent", { limit: 5, cursor: first.nextCursor });
    expect(new Set([...first.items, ...second.items].map((r) => r.ref)).size).toBe(10);
    expect(first.items[0]).not.toHaveProperty("payload");
    await expect(memory.list("agent", { limit: 51 })).rejects.toThrow("page size");
  });
  it("atomically validates links, artifacts and run identity without consuming numbers", async () => {
    const { memory } = f.services();
    await memory.publishResearch(f.analyst, "initial", researchInput(f.now));
    await expect(memory.publishResearch(f.analyst, "bad-link", researchInput(f.now, { links: [{ ref: "m999", relation: "based_on" }] }))).rejects.toThrow("not found");
    await expect(memory.publishResearch({ ...f.analyst, runId: "run-foreign" }, "foreign-run", researchInput(f.now))).rejects.toThrow("match");
    const artifacts = new ArtifactService(f.db);
    const foreign = await artifacts.create("other", "foreign", { title: "Other", mediaType: "text/plain", content: "Evidence" });
    await expect(memory.publishResearch(f.analyst, "bad-artifact", researchInput(f.now, { artifactIds: [foreign.id] }))).rejects.toThrow("Artifact");
    expect(await f.db.get("SELECT count(*) AS total FROM hv_memory_operations")).toMatchObject({ total: 1 });
    expect(await memory.publishResearch(f.analyst, "next", researchInput(f.now))).toMatchObject({ ref: "m2" });
    // A late FK failure also rolls back the counter and receipt.
    await expect(f.db.transaction(async (tx) => {
      await tx.run("UPDATE hv_memory_counters SET next_number=99 WHERE agent_id='agent'");
      await tx.run("INSERT INTO hv_memory_links(agent_id,source_number,target_number,relation) VALUES ('agent',2,999,'based_on')");
    })).rejects.toThrow();
    expect(await f.db.get("SELECT next_number FROM hv_memory_counters WHERE agent_id='agent'")).toMatchObject({ next_number: 3 });
  });
  it("allows terminal/expired exact replay but rejects new effects", async () => {
    const { memory } = f.services(), input = researchInput(f.now);
    const original = await memory.publishResearch(f.analyst, "done", input);
    await f.db.run("UPDATE hv_subagent_runs SET status='completed' WHERE id='run-analyst'");
    expect(await memory.publishResearch(f.analyst, "done", input)).toEqual(original);
    await expect(memory.publishResearch(f.analyst, "late", input)).rejects.toThrow("active");
  });
  it("selects newest before freshness, never falls back, and keeps instrument streams separate", async () => {
    const { memory } = f.services();
    await memory.publishResearch(f.analyst, "initial", researchInput(f.now));
    expect(await memory.tradingContext("agent", "BTC", researchRequirement)).toMatchObject({ requirements: [{ ref: "m1", status: "fresh", summary: "Mixed momentum near support" }] });
    await memory.publishResearch(f.analyst, "older-evidence", researchInput(f.now - 7200_000));
    expect(await memory.tradingContext("agent", "BTC", researchRequirement)).toMatchObject({ requirements: [{ ref: "m2", status: "stale", summary: null }] });
    expect(await memory.tradingContext("agent", "ETH", researchRequirement)).toMatchObject({ requirements: [{ ref: null, status: "missing" }] });
    await memory.publishResearch(f.analyst, "new-report", researchInput(f.now));
    await f.db.run("UPDATE hv_memories SET retracted_at=?,retraction_reason='Withdrawn evidence' WHERE agent_id='agent' AND number=3", f.now);
    expect(await memory.tradingContext("agent", "BTC", researchRequirement)).toMatchObject({ requirements: [{ ref: "m3", status: "retracted", summary: null }] });
    expect((await memory.list("agent", { status: "retracted" })).items.map((item) => item.ref)).toEqual(["m3"]);
    expect((await memory.get("agent", "m1")).content).toContain("Facts");
    expect((await memory.tradingContext("agent", "BTC", [])).requirements).toEqual([]);
    await expect(memory.tradingContext("agent", "BTC", Array(21).fill(researchRequirement[0]))).rejects.toThrow("20");
  });
  it.each([
    { name: "enabled", enabled: 1, archived: 0 },
    { name: "disabled", enabled: 0, archived: 0 },
    { name: "archived", enabled: 0, archived: 1 },
  ])("evaluates published research independently of an $name producer", async ({ enabled, archived }) => {
    const { memory } = f.services(), boundary = f.now;
    await memory.publishResearch(f.analyst, "initial", researchInput(boundary, { requestedValidUntil: boundary + 600_000 }));
    await f.db.run("UPDATE hv_subagents SET enabled=?,archived=? WHERE id='analyst'", enabled, archived);
    const expectStatus = async (status: string) => {
      expect(await memory.get("agent", "m1")).toMatchObject({ status });
      expect((await memory.list("agent", { status })).items.map((item) => item.ref)).toEqual(["m1"]);
      expect(await memory.tradingContext("agent", "BTC", researchRequirement)).toMatchObject({ requirements: [{ ref: "m1", status, summary: status === "fresh" ? "Mixed momentum near support" : null }] });
    };
    await expectStatus("fresh");
    expect(await memory.tradingContext("agent", "ETH", researchRequirement)).toMatchObject({ requirements: [{ ref: null, status: "missing", summary: null }] });
    expect(await memory.tradingContext("agent", "BTC", [{ ...researchRequirement[0], subagentId: "unknown" }])).toMatchObject({ requirements: [{ ref: null, status: "missing", summary: null }] });
    f.now = boundary - 1;
    await expectStatus("invalid_evidence");
    f.now = boundary + 60_000;
    expect(await memory.tradingContext("agent", "BTC", [{ ...researchRequirement[0], maxAgeSeconds: 60 }])).toMatchObject({ requirements: [{ ref: "m1", status: "stale", summary: null }] });
    await expectStatus("fresh");
    f.now = boundary + 600_000;
    await expectStatus("stale");
    await f.db.run("UPDATE hv_memories SET retracted_at=?,retraction_reason='Withdrawn evidence' WHERE agent_id='agent' AND number=1", f.now);
    await expectStatus("retracted");
  });
  it("preserves optional and legacy research data without interpreting labels as trade readiness", async () => {
    let { memory } = f.services();
    const payloads = [{}, { bias: "bearish-neutral", uncertainty: ["Support may hold", "Hourly trend mixed"] }, { version: 1, bias: "long", setup: "withdrawn", observations: ["Old report"], interpretation: "No longer valid", invalidation: [], coverage: "unknown" }];
    for (const [index, payload] of payloads.entries()) {
      const input = researchInput(f.now, { payload });
      await memory.publishResearch(f.analyst, `data:${index}`, input);
      ({ memory } = await f.reopen());
      const ref = `m${index + 1}`;
      expect((await memory.get("agent", ref)).payload).toEqual(payload);
      expect(await memory.publishResearch(f.analyst, `data:${index}`, input)).toMatchObject({ ref });
      expect((await memory.list("agent", { status: "fresh" })).items[0]).toMatchObject({ ref, status: "fresh" });
      const context = await memory.tradingContext("agent", "BTC", researchRequirement);
      expect(context.requirements[0]).toMatchObject({ ref, status: "fresh" });
      expect(context).not.toHaveProperty("allReady");
      expect(context.requirements[0]).not.toHaveProperty("eligible");
      expect(context.requirements[0]).not.toHaveProperty("payload");
    }
  });
  it("caps validity by evidence and stricter requirements; validates complete replacement corrections", async () => {
    const { memory } = f.services();
    await expect(memory.publishResearch(f.analyst, "future", researchInput(f.now + 3600_000))).rejects.toThrow("boundary");
    await expect(memory.publishResearch(f.analyst, "unaligned", researchInput(f.now - 1))).rejects.toThrow("boundary");
    const original = await memory.publishResearch(f.analyst, "original", researchInput(f.now, { requestedValidUntil: f.now + 600_000 }));
    expect(original.validUntil).toBe(f.now + 600_000);
    const corrected = await memory.publishResearch(f.analyst, "correct", researchInput(f.now, { requestedValidUntil: f.now + 3600_000, links: [{ ref: "m1", relation: "corrects" }] }));
    expect(corrected.validUntil).toBe(original.validUntil);
    expect((await memory.get("agent", "m2")).links).toEqual([{ ref: "m1", relation: "corrects" }]);
    await expect(memory.publishResearch(f.analyst, "wrong-stream", researchInput(f.now, { instrument: "ETH", links: [{ ref: "m1", relation: "supersedes" }] }))).rejects.toThrow("same research");
    f.now += 60_000;
    expect(await memory.tradingContext("agent", "BTC", [{ ...researchRequirement[0], maxAgeSeconds: 60 }])).toMatchObject({ requirements: [{ status: "stale" }] });
  });
  it("returns bounded discovery summaries, leaving large reports and custom data for get_memory", async () => {
    const { memory } = f.services();
    const requirements = [], summary = "😀".repeat(250), content = "x".repeat(16000), payload = { measurements: "x".repeat(16000) };
    for (let n = 0; n < 20; n++) {
      const topic = `stream_${n}`;
      await memory.publishResearch(f.analyst, `large:${n}`, researchInput(f.now, { topic, summary, content, payload }));
      requirements.push({ ...researchRequirement[0], subagentId: "analyst", topic, interval: "1h", maxAgeSeconds: 3600 });
    }
    const result = await memory.tradingContext("agent", "BTC", requirements);
    expect(result.requirements).toHaveLength(20);
    expect(result.requirements.every((item) => item.status === "fresh" && item.summary === summary)).toBe(true);
    expect(new TextEncoder().encode(JSON.stringify(result)).length).toBeLessThan(64 * 1024);
    for (const item of result.requirements) {
      expect(item).not.toHaveProperty("content");
      expect(item).not.toHaveProperty("payload");
    }
    expect(await memory.get("agent", "m1")).toMatchObject({ summary, content, payload });
    expect((await memory.list("agent", { query: "😀".repeat(10) })).items).toHaveLength(20);
  });
  it("records decision/review relationships with host provenance and refuses fabricated authority", async () => {
    const { memory } = f.services();
    await memory.publishResearch(f.analyst, "research", researchInput(f.now));
    const decision = { topic: "entry", instrument: "BTC", summary: "User-directed decision", content: "A complete explicit plan", payload: { version: 1, action: "enter", direction: "long", reason: "Requested", researchRefs: ["m1"], plan: "Full-size TP/SL; no claim of fills" } };
    await memory.recordDecision(f.analyst, "decision", decision);
    expect(await memory.get("agent", "m2")).toMatchObject({ sourceRunId: "run-analyst", links: [{ ref: "m1", relation: "based_on" }] });
    await expect(memory.recordDecision(f.analyst, "forged", { ...decision, payload: { ...decision.payload, origin: "user_directed" } })).rejects.toThrow("host-owned");
    await expect(memory.recordDecision(f.analyst, "non-research", { ...decision, payload: { ...decision.payload, researchRefs: ["m2"] } })).rejects.toThrow("research publications");
    await memory.publishReview(f.reviewer, "review", { topic: "daily", summary: "Sparse sample", content: "No causal claims", payload: { version: 1, start: f.now - 86400_000, end: f.now, coverage: "incomplete", limitations: ["Small sample"], findings: ["One entry decision"], recommendations: ["Continue observing"], memoryRefs: ["m1", "m2"] } });
    expect(await memory.get("agent", "m3")).toMatchObject({ sourceRunId: "run-reviewer", links: [{ ref: "m1", relation: "reviews" }, { ref: "m2", relation: "reviews" }] });
  });
  it("protects linked evidence, report attachments and tombstones through all retention paths", async () => {
    const { memory } = f.services(), artifacts = new ArtifactService(f.db);
    const child = await artifacts.create("agent", "csv", { title: "Source", mediaType: "text/csv", content: "close\n10" });
    const report = await artifacts.create("agent", "report", { title: "Evidence", mediaType: "text/markdown", content: `Facts artifact:${child.id}` });
    // Publication uses the injected time; artifacts have ordinary real-clock TTLs.
    await memory.publishResearch(f.analyst, "evidence", researchInput(f.now, { artifactIds: [report.id] }));
    await f.db.run("UPDATE hv_artifacts SET expires_at=0");
    await artifacts.cleanup();
    expect((await artifacts.get("agent", report.id)).id).toBe(report.id);
    expect((await artifacts.get("agent", child.id)).id).toBe(child.id);
    expect((await artifacts.list("agent")).items).toHaveLength(2);
    for (const id of [report.id, child.id]) {
      await expect(artifacts.remove("agent", id)).rejects.toThrow("memory");
      await expect(artifacts.retention("agent", id, false)).rejects.toThrow("memory");
    }
    const expired = await artifacts.create("agent", "expired", { title: "Expired", mediaType: "text/plain", content: "Gone" });
    await f.db.run("UPDATE hv_artifacts SET expires_at=0 WHERE id=?", expired.id);
    await expect(memory.publishResearch(f.analyst, "resurrect", researchInput(f.now, { artifactIds: [expired.id] }))).rejects.toThrow("expired");
    await artifacts.cleanup();
    await expect(artifacts.create("agent", "expired", { title: "Expired", mediaType: "text/plain", content: "Gone" })).rejects.toThrow("unavailable");
    expect(await f.db.get("SELECT count(*) AS total FROM hv_memory_artifacts")).toMatchObject({ total: 1 });
  });
});

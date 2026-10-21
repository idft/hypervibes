import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { hvUsers } from "../db/auth-schema.js";
import { newDrawing, type ChartCapture, type ChartLayout } from "../charts/contracts.js";

describe("root chart conversations", () => {
  let directory: string, runtime: AppRuntime, faux: ReturnType<typeof fauxProvider>, agentId: string, layout: ChartLayout;
  const capture = (): ChartCapture => ({ layoutId: layout.id, revision: layout.revision, status: "live", capturedAt: Date.now(), range: { from: 3600, to: 7200 }, candles: [{ time: 3600, open: 100, high: 110, low: 95, close: 105, volume: 10 }] });
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-chart-chat-"));
    faux = fauxProvider({ models: [{ id: "vision", input: ["text", "image"] }, { id: "text", input: ["text"] }] });
    const models = createModels(); models.setProvider(faux.provider);
    runtime = await createRuntime(testConfig({ APP_SQLITE_PATH: join(directory, "test.sqlite") }), {
      models, model: { provider: faux.provider.id, modelId: "vision" }, accountFeedFactory: () => ({ close: async () => {} }), journalFeedFactory: () => ({ close: async () => {} }), journalHistoryFetcher: async () => [],
    });
    await runtime.db.insert(hvUsers).values({ id: "owner", name: "Owner", email: "owner@example.test" }).run(); runtime.start();
    agentId = (await runtime.agents.create("owner", "Chart Agent", randomUUID(), `0x${"a".repeat(40)}`)).id;
    layout = await runtime.agents.charts.create(agentId, randomUUID(), { name: "BTC study", coin: "BTC", interval: "1h" });
  });
  afterEach(async () => { await runtime.close(); rmSync(directory, { recursive: true, force: true }); });
  const finished = async (text: string) => vi.waitFor(async () => {
    const chat = await runtime.agents.chat("owner", agentId);
    expect(chat.busy).toBe(false); expect(chat.messages.some((entry) => entry.text === text)).toBe(true);
  });
  it("attaches frozen structured context and shows only the operator's actual text in chat/history", async () => {
    const id = randomUUID(), input = capture();
    faux.setResponses([(context) => {
      const text = JSON.stringify(context.messages);
      expect(text).toContain(id); expect(text).toContain("BTC study"); expect(text).toContain("get_chart_snapshot"); expect(text).toContain("not live");
      return fauxAssistantMessage("We are discussing BTC.");
    }]);
    await runtime.agents.send("owner", agentId, "Explain this", id, input); await finished("We are discussing BTC.");
    const chat = await runtime.agents.chat("owner", agentId);
    expect(chat.messages.find((entry) => entry.role === "user")).toMatchObject({ text: "Explain this", chart: { snapshotId: id, coin: "BTC", interval: "1h", image: false } });
    expect(chat.supportsImages).toBe(true);
    await runtime.agents.send("owner", agentId, "Explain this", id, input);
    expect(faux.state.callCount).toBe(1);
    expect(await runtime.database.all("SELECT * FROM hv_memories")).toEqual([]);
    expect(await runtime.database.all("SELECT * FROM hv_worker_requests")).toEqual([]);
    await expect(runtime.agents.send("owner", agentId, "Changed message", id, input)).rejects.toThrow("different content");
    await expect(runtime.agents.send("someone-else", agentId, "Explain", randomUUID(), input)).rejects.toThrow("not found");
  });
  it("gives the root actual image content and rejects switching to text-only until context reset", async () => {
    const input = { ...capture(), image: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jGJkAAAAASUVORK5CYII=" };
    faux.setResponses([(context) => {
      const content = context.messages.filter((message) => message.role === "user").at(-1)!.content;
      expect(content).toEqual(expect.arrayContaining([{ type: "image", data: input.image, mimeType: "image/png" }]));
      return fauxAssistantMessage("Image received");
    }]);
    await runtime.agents.send("owner", agentId, "See chart", randomUUID(), input); await finished("Image received");
    const settings = { provider: faux.provider.id, modelId: "text", thinkingLevel: "" };
    await expect(runtime.agents.subagents.updateSection("owner", agentId, `main:${agentId}`, "model", settings)).rejects.toThrow("Reset context");
    await runtime.agents.reset("owner", agentId);
    await runtime.agents.subagents.updateSection("owner", agentId, `main:${agentId}`, "model", settings);
    expect((await runtime.agents.chat("owner", agentId)).supportsImages).toBe(false);
    await expect(runtime.agents.send("owner", agentId, "See image", randomUUID(), input)).rejects.toThrow("cannot accept chart images");
    expect(faux.state.callCount).toBe(1);
  });
  it("queues the captured view, not whichever instrument is selected when work resumes", async () => {
    let release!: () => void;
    const held = new Promise<void>((resolve) => { release = resolve; });
    faux.setResponses([async () => { await held; return fauxAssistantMessage("First finished"); }, (context) => {
      expect(JSON.stringify(context.messages)).toContain("BTC study"); return fauxAssistantMessage("Frozen BTC reviewed");
    }]);
    await runtime.agents.send("owner", agentId, "First", randomUUID());
    await vi.waitFor(() => expect(faux.state.callCount).toBe(1));
    const id = randomUUID(), input = capture();
    await runtime.agents.send("owner", agentId, "Review the chart", id, input);
    const queued = (await runtime.agents.chat("owner", agentId)).queuedMessages;
    expect(queued).toEqual([expect.objectContaining({ text: "Review the chart", chart: expect.objectContaining({ coin: "BTC", snapshotId: id }) })]);
    layout = await runtime.agents.charts.edit(agentId, layout.id, 0, { label: "Switch", settings: { coin: "ETH" } }, "switch");
    await runtime.agents.send("owner", agentId, "Review the chart", id, input);
    release(); await finished("Frozen BTC reviewed");
    expect((await runtime.agents.charts.snapshot(agentId, id)).document.coin).toBe("BTC");
    expect(faux.state.callCount).toBe(2);
  });
  it("lets the orchestrator inspect and immediately draw one undoable batch without publishing research", async () => {
    const snapshotId = randomUUID(), drawing = { ...newDrawing("horizontal", "BTC", [{ time: 3600, price: 110 }]), author: "agent" as const, name: "Prior high" };
    faux.setResponses([
      fauxAssistantMessage(fauxToolCall("get_chart_snapshot", { snapshotId }), { stopReason: "toolUse" }),
      fauxAssistantMessage(fauxToolCall("get_chart_layout", { layoutId: layout.id }), { stopReason: "toolUse" }),
      fauxAssistantMessage(fauxToolCall("edit_chart_annotations", { layoutId: layout.id, expectedRevision: 0, label: "Resistance", upsert: [{ ...drawing, points: drawing.points.map((p) => ({ ...p })) }], notes: "Study the reaction; not a trade." }), { stopReason: "toolUse" }),
      (context) => {
        const results = context.messages.filter((message) => message.role === "toolResult");
        expect(results).toHaveLength(3); expect(results.every((r) => !r.isError)).toBe(true);
        return fauxAssistantMessage("Resistance marked");
      },
    ]);
    await runtime.agents.send("owner", agentId, "Mark the prior high", snapshotId, capture()); await finished("Resistance marked");
    const updated = await runtime.agents.charts.get(agentId, layout.id);
    expect(updated.document.drawings).toEqual([drawing]); expect(updated.document.notes).toContain("not a trade"); expect(updated.revision).toBe(1);
    const undone = await runtime.agents.charts.undo(agentId, layout.id, 1, "undo-agent");
    expect(undone.document.drawings).toEqual([]); expect(undone.document.notes).toBe("");
    expect(await runtime.database.all("SELECT * FROM hv_memories")).toEqual([]);
    expect(await runtime.database.all("SELECT * FROM hv_order_requests")).toEqual([]);
    expect(await runtime.database.all("SELECT * FROM hv_insight_sets")).toEqual([]);
  });
});

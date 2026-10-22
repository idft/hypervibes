import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { openDatabase } from "../db/database.js";
import { migrateApplication } from "../db/migrations.js";
import { SharedDatabase } from "../db/shared-database.js";
import { ChartService } from "./chart-service.js";
import { chartCapture, chartEdit, newDrawing, visibleDrawing, type ChartCapture, type ChartLayout } from "./contracts.js";
import { lineEnds } from "./geometry.js";

describe("persistent collaborative chart layouts", () => {
  let directory: string, db: SharedDatabase, charts: ChartService, layout: ChartLayout;
  const path = () => join(directory, "test.sqlite");
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-charts-"));
    const connection = openDatabase(path()); migrateApplication(connection); db = new SharedDatabase(connection); charts = new ChartService(db);
    await db.run("INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test')");
    for (const id of ["a", "b"]) await db.run("INSERT INTO hv_agents(id,user_id,name,creation_request_id,created_at,updated_at) VALUES (?,'owner',?,?,0,0)", id, id, id);
    layout = await charts.create("a", randomUUID(), { name: "BTC structure", coin: "BTC", interval: "1h" });
  });
  afterEach(async () => { await db.close(); rmSync(directory, { recursive: true, force: true }); });
  const capture = (): ChartCapture => ({ layoutId: layout.id, revision: layout.revision, capturedAt: Date.now(), status: "live", range: { from: 3600, to: 7200 }, candles: [{ time: 3600, open: 100, high: 110, low: 90, close: 105, volume: 20 }] });

  it("autosaves layouts, instrument-specific drawings and notes across reopen; duplicates with fresh identities", async () => {
    const d = newDrawing("rectangle", "BTC", [{ time: 3600, price: 100 }, { time: 7200, price: 110 }]);
    layout = await charts.edit("a", layout.id, 0, { label: "Add support", upsert: [d], settings: { notes: "Local hypothesis", interval: "4h" } }, "first");
    await db.close(); db = new SharedDatabase(openDatabase(path())); charts = new ChartService(db);
    expect(await charts.get("a", layout.id)).toEqual(layout);
    const copy = await charts.create("a", randomUUID(), { name: "Alternative" }, layout.id);
    expect(copy.document).toMatchObject({ name: "Alternative", coin: "BTC", interval: "4h", notes: "Local hypothesis" });
    expect(copy.document.drawings[0]!.id).not.toBe(d.id);
    expect(visibleDrawing(d, { ...layout.document, coin: "ETH" })).toBe(false);
    expect(visibleDrawing(d, layout.document)).toBe(true);
    expect(visibleDrawing({ ...d, intervals: ["1h"] }, layout.document)).toBe(false);
    expect((await charts.list("a"))).toHaveLength(2);
    expect(await db.all("SELECT * FROM hv_memories")).toEqual([]);
  });
  it("rejects stale revisions, isolates Agents, and makes mutation retries idempotent", async () => {
    const edit = { label: "Rename", settings: { name: "New name" } };
    const one = await charts.edit("a", layout.id, 0, edit, "same");
    expect(await charts.edit("a", layout.id, 0, edit, "same")).toEqual(one);
    await expect(charts.edit("a", layout.id, 0, { label: "Other" }, "other")).rejects.toThrow("changed elsewhere");
    await expect(charts.edit("a", layout.id, 1, edit, "same")).rejects.toThrow("different content");
    await expect(charts.get("b", layout.id)).rejects.toThrow("not found");
    await expect(charts.edit("b", layout.id, 1, edit, "foreign")).rejects.toThrow("not found");
    await expect(charts.delete("a", layout.id, 0)).rejects.toThrow("Reload");
  });
  it("undoes agent batches without undoing unrelated earlier user work, persists redo and rejects stale undo", async () => {
    const a = newDrawing("horizontal", "BTC", [{ time: 3600, price: 100 }]), b = newDrawing("text", "BTC", [{ time: 7200, price: 105 }]);
    layout = await charts.edit("a", layout.id, 0, { label: "My level", upsert: [a] }, "user");
    layout = await charts.edit("a", layout.id, 1, { label: "Scenario", upsert: [b], settings: { notes: "Possible resistance" } }, "agent", "agent");
    expect(layout.document.drawings[1]!.author).toBe("agent");
    layout = await charts.undo("a", layout.id, 2, "undo");
    expect(layout.document.drawings).toEqual([a]); expect(layout.document.notes).toBe(""); expect(layout.canRedo).toBe(true);
    await expect(charts.undo("a", layout.id, 2, "stale")).rejects.toThrow("changed elsewhere");
    await db.close(); db = new SharedDatabase(openDatabase(path())); charts = new ChartService(db);
    layout = await charts.undo("a", layout.id, 3, "redo", true);
    expect(layout.document.drawings).toHaveLength(2);
    layout = await charts.undo("a", layout.id, 4, "again");
    layout = await charts.edit("a", layout.id, 5, { label: "New branch", settings: { notes: "Different" } }, "branch");
    expect(layout.canRedo).toBe(false);
  });
  it("enforces locks for both authors, preserves provenance, and limits agents to annotations/notes", async () => {
    const d = { ...newDrawing("horizontal", "BTC", [{ time: 3600, price: 100 }]), locked: true };
    layout = await charts.edit("a", layout.id, 0, { label: "Lock", upsert: [d] }, "lock");
    await expect(charts.edit("a", layout.id, 1, { label: "Delete", remove: [d.id] }, "delete", "agent")).rejects.toThrow("Unlock");
    await expect(charts.edit("a", layout.id, 1, { label: "Unlock", upsert: [{ ...d, locked: false }] }, "unlock-agent", "agent")).rejects.toThrow("Unlock");
    await expect(charts.edit("a", layout.id, 1, { label: "Move", upsert: [{ ...d, points: [{ time: 3600, price: 200 }] }] }, "move")).rejects.toThrow("Unlock");
    layout = await charts.edit("a", layout.id, 1, { label: "Unlock", upsert: [{ ...d, locked: false }] }, "unlock-user");
    layout = await charts.edit("a", layout.id, 2, { label: "Label", upsert: [{ ...d, locked: false, author: "agent", text: "Support" }] }, "label", "agent");
    expect(layout.document.drawings[0]!.author).toBe("user");
    await expect(charts.edit("a", layout.id, 3, { label: "Switch", settings: { coin: "ETH" } }, "switch", "agent")).rejects.toThrow("not change");
  });
  it("freezes image, viewport, annotations and candles across message retries and later edits/deletion", async () => {
    const id = randomUUID(), input = capture();
    const first = await charts.capture("a", id, "Explain this", input);
    layout = await charts.edit("a", layout.id, 0, { label: "Switch", settings: { coin: "ETH" } }, "switch");
    expect(await charts.capture("a", id, "Explain this", input)).toEqual(first);
    await expect(charts.capture("a", id, "Explain this", { ...input, status: "reconnecting" })).rejects.toThrow("different content");
    await expect(charts.capture("a", randomUUID(), "Explain", input)).rejects.toThrow("changed before sending");
    await expect(charts.snapshot("b", id)).rejects.toThrow("not found");
    await charts.delete("a", layout.id, 1);
    const saved = await charts.snapshot("a", id, 0, 1);
    expect(saved.document.coin).toBe("BTC"); expect(saved.candles).toEqual(input.candles); expect(saved.nextOffset).toBeNull();
  });
  it("validates images, bounded geometry, duplicate IDs and snapshot ordering", async () => {
    const input = capture();
    await expect(charts.capture("a", randomUUID(), "Image", { ...input, image: "invalid" })).rejects.toThrow("PNG");
    const png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jGJkAAAAASUVORK5CYII=";
    expect((await charts.capture("a", randomUUID(), "Image", { ...input, image: png })).attachment.image).toBe(true);
    expect(() => chartCapture({ ...input, candles: [input.candles[0], input.candles[0]] })).toThrow("ordering");
    const d = newDrawing("horizontal", "BTC", [{ time: 3600, price: 100 }]);
    expect(() => chartEdit({ label: "Bad", upsert: [d, d] })).toThrow("duplicate");
    expect(() => chartEdit({ label: "Bad", upsert: [{ ...d, color: "url(evil)" }] })).toThrow("color");
    expect(() => chartEdit({ label: "Bad", upsert: [{ ...d, points: [{ time: 0, price: NaN }] }] })).toThrow("number");
    expect(lineEnds("extended", { x: 20, y: 30 }, { x: 40, y: 50 }, 100)).toEqual([{ x: 0, y: 10 }, { x: 100, y: 110 }]);
    expect(lineEnds("ray", { x: 40, y: 50 }, { x: 20, y: 30 }, 100)).toEqual([{ x: 40, y: 50 }, { x: 0, y: 10 }]);
  });
  it("cascades application-owned layouts, receipts and snapshots on Agent deletion", async () => {
    await charts.capture("a", randomUUID(), "Discuss", capture());
    await db.run("DELETE FROM hv_agents WHERE id='a'");
    for (const table of ["hv_chart_layouts", "hv_chart_operations", "hv_chart_snapshots"]) expect(await db.all(`SELECT * FROM ${table}`)).toEqual([]);
    expect(await db.all("PRAGMA foreign_key_check")).toEqual([]);
  });
  it("normalizes legacy documents, validates instance ownership and duplicates with fresh identities", async () => {
    await db.run("UPDATE hv_chart_layouts SET document=? WHERE id=?", JSON.stringify({ name: "Legacy", coin: "BTC", interval: "1h", drawings: [], notes: "", hideDrawings: false, magnet: true }), layout.id);
    expect((await charts.get("a", layout.id)).document.indicators).toEqual([]);
    // A foreign custom indicator in a forged document is rejected before any edit is stored.
    const foreign = { id: randomUUID(), indicatorId: "custom:12345678-1234-4234-8234-123456789abc", inputs: {}, configuredVersion: 1, hidden: false, pane: "auto", styles: {} };
    await expect(charts.edit("a", layout.id, 0, { label: "Forge", settings: { indicators: [foreign] } }, "forge")).rejects.toThrow("not found in this Agent");
    await expect(charts.create("a", randomUUID(), { indicators: [{ indicatorId: "builtin:nope" }] })).rejects.toThrow();
    const own = await db.run("INSERT INTO hv_indicators(id,agent_id,name,description,interpretation,current_version,created_at,updated_at) VALUES ('12345678-1234-4234-8234-123456789abc','a','Mine','','',1,1,1)");
    expect(own).toBeUndefined();
    const saved = await charts.edit("a", layout.id, 0, { label: "Add indicator", settings: { indicators: [foreign] } }, "add");
    expect(saved.document.indicators).toEqual([foreign]);
    const copy = await charts.create("a", randomUUID(), {}, layout.id);
    expect(copy.document.indicators).toHaveLength(1);
    expect(copy.document.indicators[0]!.id).not.toBe(foreign.id);
    expect(copy.document.indicators[0]!.indicatorId).toBe(foreign.indicatorId);
    await db.run("DELETE FROM hv_agents WHERE id='a'");
    expect(await db.all("SELECT * FROM hv_indicators")).toEqual([]);
  });
});

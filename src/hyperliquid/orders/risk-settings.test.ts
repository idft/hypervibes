import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { memoryFixture } from "../../memory/test-fixture.js";
import { RiskSettingsService, riskSettingsInput } from "./risk-settings.js";

const defaults = { tradingEnabled: false, allowedInstruments: null, maxOrderNotional: null, maxTotalNotional: null };
describe("per-Agent risk settings", () => {
  let f: Awaited<ReturnType<typeof memoryFixture>>;
  beforeEach(async () => { f = await memoryFixture(); });
  afterEach(async () => { await f.close(); });
  it("defaults to disabled and keeps independent settings across reopen", async () => {
    let service = new RiskSettingsService(f.db);
    expect(await service.get("agent")).toEqual(defaults);
    const input = { tradingEnabled: true, allowedInstruments: ["BTC", "ETH"], maxOrderNotional: "100.001", maxTotalNotional: "1000" };
    expect(await service.save("agent", input)).toEqual(input);
    expect(await service.get("other")).toEqual(defaults);
    await f.reopen(); service = new RiskSettingsService(f.db);
    expect(await service.get("agent")).toEqual(input);
    expect(await service.save("agent", defaults)).toEqual(defaults);
  });
  it("notifies once for each successful save, after the transaction commits", async () => {
    let committed = false;
    const transaction = f.db.transaction.bind(f.db);
    vi.spyOn(f.db, "transaction").mockImplementation(async (callback) => {
      const result = await transaction(callback);
      committed = true;
      return result;
    });
    const changed = vi.fn((id: string) => {
      expect(committed).toBe(true);
      expect(id).toBe("agent");
    });
    const service = new RiskSettingsService(f.db, changed);
    const input = { ...defaults, tradingEnabled: true, maxOrderNotional: "100" };
    expect(await service.save("agent", input)).toEqual(input);
    expect(changed).toHaveBeenCalledTimes(1);
    expect(await service.get("agent")).toEqual(input);
    committed = false;
    await service.save("agent", input);
    expect(changed).toHaveBeenCalledTimes(2);
  });
  it("rejects invalid/duplicate symbols, nonpositive or numeric amounts and unknown fields", () => {
    for (const patch of [{ tradingEnabled: "true" }, { allowedInstruments: [] }, { allowedInstruments: ["BTC", "BTC"] }, { allowedInstruments: [" BTC"] }, { allowedInstruments: ["xyz:BTC"] }, { allowedInstruments: Array(51).fill("BTC") }, { maxOrderNotional: 100 }, { maxOrderNotional: "0" }, { maxTotalNotional: "1e3" }, { maxTotalNotional: "-100" }, { automatic: true }]) expect(() => riskSettingsInput({ ...defaults, ...patch })).toThrow();
    expect(riskSettingsInput({ ...defaults, maxOrderNotional: "", maxTotalNotional: "" })).toEqual(defaults);
  });
  it("does not save partial settings on failure or invent unknown Agent ownership", async () => {
    const changed = vi.fn();
    const service = new RiskSettingsService(f.db, changed);
    await expect(service.save("agent", { ...defaults, maxTotalNotional: "bad" })).rejects.toThrow();
    expect(await service.get("agent")).toEqual(defaults);
    await expect(service.save("foreign", defaults)).rejects.toThrow();
    for (const state of ["stopping", "stopped"]) {
      await f.db.run("UPDATE hv_agents SET state=? WHERE id='agent'", state);
      await expect(service.save("agent", defaults)).rejects.toMatchObject({ status: 409 });
    }
    expect(changed).not.toHaveBeenCalled();
  });
  it("does not notify when the database rejects a save", async () => {
    const changed = vi.fn();
    const service = new RiskSettingsService(f.db, changed);
    await service.save("agent", { ...defaults, maxOrderNotional: "100" });
    changed.mockClear();
    await f.db.exec("CREATE TEMP TRIGGER reject_risk_save BEFORE UPDATE ON hv_risk_settings BEGIN SELECT RAISE(ABORT, 'risk write failed'); END");
    await expect(service.save("agent", { ...defaults, maxOrderNotional: "200" })).rejects.toThrow("risk write failed");
    expect(changed).not.toHaveBeenCalled();
    expect(await service.get("agent")).toEqual({ ...defaults, maxOrderNotional: "100" });
  });
});

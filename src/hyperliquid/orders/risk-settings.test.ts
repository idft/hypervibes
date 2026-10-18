import { afterEach, beforeEach, describe, expect, it } from "vitest";
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
  it("rejects invalid/duplicate symbols, nonpositive or numeric amounts and unknown fields", () => {
    for (const patch of [{ tradingEnabled: "true" }, { allowedInstruments: [] }, { allowedInstruments: ["BTC", "BTC"] }, { allowedInstruments: [" BTC"] }, { allowedInstruments: ["xyz:BTC"] }, { allowedInstruments: Array(51).fill("BTC") }, { maxOrderNotional: 100 }, { maxOrderNotional: "0" }, { maxTotalNotional: "1e3" }, { maxTotalNotional: "-100" }, { automatic: true }]) expect(() => riskSettingsInput({ ...defaults, ...patch })).toThrow();
    expect(riskSettingsInput({ ...defaults, maxOrderNotional: "", maxTotalNotional: "" })).toEqual(defaults);
  });
  it("does not save partial settings on failure or invent unknown Agent ownership", async () => {
    const service = new RiskSettingsService(f.db);
    await expect(service.save("agent", { ...defaults, maxTotalNotional: "bad" })).rejects.toThrow();
    expect(await service.get("agent")).toEqual(defaults);
    await expect(service.save("foreign", defaults)).rejects.toThrow();
  });
});

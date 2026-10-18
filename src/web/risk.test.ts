import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppRuntime } from "../runtime.js";
import { AgentError } from "../agents/agent-input.js";
const mocks = vi.hoisted(() => ({ currentUser: vi.fn(), headers: vi.fn() }));
vi.mock("../auth-user.js", () => ({ currentUser: mocks.currentUser }));
vi.mock("@tanstack/react-start/server", () => ({ getRequestHeaders: mocks.headers }));
vi.mock("@tanstack/react-start", () => {
  const builder = (validate: (data: unknown) => unknown = (data) => data) => ({
    validator: (next: (data: unknown) => unknown) => builder(next),
    handler: (handler: (args: { context: { runtime: AppRuntime }; data: unknown }) => Promise<unknown>) => ({ invoke: (runtime: AppRuntime, input: unknown) => handler({ context: { runtime }, data: validate(input) }) }),
  });
  return { createServerFn: () => builder() };
});
import { getRiskSettings, saveRiskSettings } from "./risk.js";
function invoke(fn: unknown, runtime: AppRuntime, input: unknown) { return (fn as { invoke(runtime: AppRuntime, input: unknown): Promise<unknown> }).invoke(runtime, input); }
describe("authenticated risk management settings", () => {
  const owned = vi.fn(), get = vi.fn(), save = vi.fn();
  let runtime: AppRuntime;
  beforeEach(() => {
    vi.resetAllMocks(); mocks.currentUser.mockResolvedValue({ id: "owner" }); mocks.headers.mockReturnValue(new Headers({ origin: "http://localhost:3003" }));
    owned.mockImplementation(async (_user: string, id: string) => { if (id !== "agent") throw new AgentError("Agent not found", 404); });
    get.mockResolvedValue({ tradingEnabled: false }); save.mockResolvedValue({ tradingEnabled: true });
    runtime = { origin: "http://localhost:3003", status: vi.fn(), agents: { owned, risk: { get, save } } } as unknown as AppRuntime;
  });
  it("checks Agent ownership before reads or writes", async () => {
    expect(await invoke(getRiskSettings, runtime, { agentId: "agent" })).toEqual({ ok: true, data: { tradingEnabled: false } });
    expect(await invoke(saveRiskSettings, runtime, { agentId: "foreign", settings: {} })).toMatchObject({ ok: false, error: "Agent not found" });
    expect(save).not.toHaveBeenCalled();
    const settings = { tradingEnabled: true, allowedInstruments: ["BTC"], maxOrderNotional: null, maxTotalNotional: "100" };
    await invoke(saveRiskSettings, runtime, { agentId: "agent", settings });
    expect(save).toHaveBeenCalledWith("agent", settings);
  });
  it("rejects cross-origin mutations and unauthenticated access", async () => {
    mocks.headers.mockReturnValue(new Headers({ origin: "https://evil.example" }));
    await expect(invoke(saveRiskSettings, runtime, { agentId: "agent", settings: {} })).rejects.toMatchObject({ status: 403 });
    expect(save).not.toHaveBeenCalled();
    mocks.currentUser.mockResolvedValue(null);
    await expect(invoke(getRiskSettings, runtime, { agentId: "agent" })).rejects.toBeDefined();
    expect(get).not.toHaveBeenCalled();
  });
});

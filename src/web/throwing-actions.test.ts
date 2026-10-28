import { beforeEach, describe, expect, it, vi } from "vitest";
import { isRedirect } from "@tanstack/react-router";
import { AgentError } from "../agents/agent-input.js";
import { HyperliquidError } from "../hyperliquid/errors.js";
import type { AppRuntime } from "../runtime.js";
const mocks = vi.hoisted(() => ({ user: vi.fn(), headers: vi.fn() }));
vi.mock("../auth-user.js", () => ({ currentUser: mocks.user }));
vi.mock("@tanstack/react-start/server", () => ({ getRequestHeaders: mocks.headers }));
vi.mock("@tanstack/react-start", () => {
  const builder = (validate: (data: unknown) => unknown = (data) => data) => ({
    validator: (next: (data: unknown) => unknown) => builder(next),
    handler: (handler: (args: { context: { runtime: AppRuntime }; data: unknown }) => Promise<unknown>) => ({ invoke: (runtime: AppRuntime, input: unknown) => handler({ context: { runtime }, data: validate(input) }) }),
  });
  return { createServerFn: () => builder() };
});
import { cancelAgentOrder, closeAgentPosition, setAgentEnabled } from "./agents.js";
import { beginProviderLogin, getModelChoices, getProviderSettings, refreshProviderModels } from "./providers.js";
import { approveApiWallet, getAccount, setupApiWallet } from "./account.js";
import { listSubagents, saveSubagentSection } from "./subagents.js";
import { readArtifact, changeArtifact } from "./artifacts.js";
import { getChartLayouts, changeChartLayout } from "./charts.js";
function invoke(fn: unknown, runtime: AppRuntime, input?: unknown) {
  return (fn as { invoke(runtime: AppRuntime, input: unknown): Promise<unknown> }).invoke(runtime, input);
}
describe("throwing domain server functions", () => {
  const settings = vi.fn(), choices = vi.fn(), begin = vi.fn(), refresh = vi.fn();
  const accountView = vi.fn(), storeApiWallet = vi.fn(), approve = vi.fn();
  let runtime: AppRuntime;
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.user.mockResolvedValue({ id: "owner", walletAddress: "0xfixture" });
    mocks.headers.mockReturnValue(new Headers({ origin: "http://localhost:3003" }));
    runtime = { origin: "http://localhost:3003", status: vi.fn(), providers: { settings, choices, begin, refresh }, hyperliquid: { accountView, storeApiWallet, approveApiWallet: approve } } as unknown as AppRuntime;
  });
  it("returns domain data directly and scopes provider access to the authenticated user", async () => {
    settings.mockResolvedValue({ providers: [], models: [], storageReady: true }); choices.mockResolvedValue([]);
    begin.mockResolvedValue({ id: "login", state: "pending" });
    expect(await invoke(getProviderSettings, runtime)).toEqual({ providers: [], models: [], storageReady: true });
    expect(await invoke(getModelChoices, runtime)).toEqual([]);
    expect(await invoke(beginProviderLogin, runtime, { providerId: "test", type: "oauth" })).toEqual({ id: "login", state: "pending" });
    expect(settings).toHaveBeenCalledWith("owner"); expect(choices).toHaveBeenCalledWith("owner");
    expect(begin).toHaveBeenCalledWith("owner", "test", "oauth");
  });
  it("sanitizes internal read and mutation failures but preserves domain errors", async () => {
    const privateError = new Error("private credential SQL details");
    settings.mockRejectedValue(privateError); refresh.mockRejectedValue(privateError); accountView.mockRejectedValue(privateError);
    for (const fn of [getProviderSettings, refreshProviderModels, getAccount]) {
      await expect(invoke(fn, runtime)).rejects.toMatchObject({ message: "Agent operation failed. Please try again.", status: 500 });
    }
    refresh.mockRejectedValue(new AgentError("Previous models were kept.", 409));
    await expect(invoke(refreshProviderModels, runtime)).rejects.toMatchObject({ message: "Previous models were kept.", status: 409 });
  });
  it("preserves account domain rejections and returns generated addresses without wrappers", async () => {
    storeApiWallet.mockResolvedValue("0xsigner");
    expect(await invoke(setupApiWallet, runtime, { source: "generate" })).toBe("0xsigner");
    expect(storeApiWallet).toHaveBeenCalledWith("owner", "generate", "");
    const error = new HyperliquidError("Signature rejected", 400);
    approve.mockRejectedValue(error);
    await expect(invoke(approveApiWallet, runtime, {})).rejects.toBe(error);
  });
  it("rejects invalid auth types and cross-origin mutations before service calls", async () => {
    await expect(invoke(beginProviderLogin, runtime, { providerId: "test", type: "ambient" })).rejects.toThrow("Invalid authentication method");
    expect(begin).not.toHaveBeenCalled();
    mocks.headers.mockReturnValue(new Headers({ origin: "https://evil.example" }));
    await expect(invoke(refreshProviderModels, runtime)).rejects.toMatchObject({ status: 403 });
    await expect(invoke(setupApiWallet, runtime, { source: "generate" })).rejects.toMatchObject({ status: 403 });
    expect(refresh).not.toHaveBeenCalled(); expect(storeApiWallet).not.toHaveBeenCalled();
  });
  it("keeps sub-agent identity and ownership checks at the service boundary", async () => {
    const list = vi.fn().mockResolvedValue([{ id: "worker" }]);
    const updateSection = vi.fn().mockRejectedValue(new AgentError("Stop the active assignment first", 409));
    const app = { ...runtime, agents: { subagents: { list, updateSection } } } as unknown as AppRuntime;
    expect(await invoke(listSubagents, app, { agentId: "agent" })).toEqual([{ id: "worker" }]);
    expect(list).toHaveBeenCalledWith("owner", "agent");
    await expect(invoke(saveSubagentSection, app, { agentId: "agent", id: "worker", section: "enabled", settings: { enabled: false } })).rejects.toMatchObject({ message: "Stop the active assignment first", status: 409 });
    expect(updateSection).toHaveBeenCalledWith("owner", "agent", "worker", "enabled", { enabled: false });
  });
  it("checks artifact/chart ownership before reading or mutating their services", async () => {
    const owned = vi.fn().mockRejectedValue(new AgentError("Agent not found", 404));
    const read = vi.fn(), retention = vi.fn(), list = vi.fn(), edit = vi.fn();
    const app = { ...runtime, agents: { owned, research: { artifacts: { read, retention } }, charts: { list, edit } } } as unknown as AppRuntime;
    const uuid = "11111111-1111-4111-8111-111111111111";
    for (const [fn, input] of [
      [readArtifact, { agentId: "foreign", id: "artifact", offset: 10 }],
      [changeArtifact, { agentId: "foreign", id: "artifact", action: "retain" }],
      [getChartLayouts, { id: "foreign" }],
      [changeChartLayout, { id: "foreign", layoutId: uuid, revision: 0, requestId: uuid, action: "edit", edit: {} }],
    ] as const) await expect(invoke(fn, app, input)).rejects.toMatchObject({ message: "Agent not found", status: 404 });
    expect(read).not.toHaveBeenCalled(); expect(retention).not.toHaveBeenCalled();
    expect(list).not.toHaveBeenCalled(); expect(edit).not.toHaveBeenCalled();
    owned.mockResolvedValue({ id: "agent" });
    read.mockResolvedValue({ id: "artifact", content: "Evidence", nextOffset: null });
    expect(await invoke(readArtifact, app, { agentId: "agent", id: "artifact", offset: 10 })).toMatchObject({ content: "Evidence" });
    expect(read).toHaveBeenCalledWith("agent", "artifact", 10);
    expect(await invoke(changeArtifact, app, { agentId: "agent", id: "artifact", action: "retain" })).toEqual({ updated: true });
    expect(retention).toHaveBeenCalledWith("agent", "artifact", true);
    list.mockRejectedValue(new Error("private SQL"));
    await expect(invoke(getChartLayouts, app, { id: "agent" })).rejects.toMatchObject({ message: "Agent operation failed. Please try again.", status: 500 });
    const conflict = new AgentError("Chart changed elsewhere", 409);
    edit.mockRejectedValue(conflict);
    await expect(invoke(changeChartLayout, app, { id: "agent", layoutId: uuid, revision: 0, requestId: uuid, action: "edit", edit: {} })).rejects.toBe(conflict);
  });
  it("authenticates stop/close actions and checks position-close ownership before signing", async () => {
    const owned = vi.fn().mockRejectedValue(new AgentError("Agent not found", 404));
    const setEnabled = vi.fn(), closePosition = vi.fn();
    const app = { ...runtime, agents: { owned, setEnabled }, orders: { closePosition } } as unknown as AppRuntime;
    expect(() => invoke(setAgentEnabled, app, { id: "agent", enabled: "false" })).toThrow("Invalid request");
    await expect(invoke(closeAgentPosition, app, { id: "foreign", symbol: "BTC", requestId: "close-1" })).rejects.toMatchObject({ status: 404 });
    expect(closePosition).not.toHaveBeenCalled();
    owned.mockResolvedValue({ id: "agent", enabled: false });
    closePosition.mockResolvedValue({ complete: true });
    expect(await invoke(closeAgentPosition, app, { id: "agent", symbol: "BTC", requestId: "close-1", size: "999", side: "buy" })).toEqual({ complete: true });
    expect(closePosition).toHaveBeenCalledWith("owner", "agent", "BTC", "close-1");
    await invoke(setAgentEnabled, app, { id: "agent", enabled: false });
    expect(setEnabled).toHaveBeenCalledWith("owner", "agent", false);
    mocks.headers.mockReturnValue(new Headers({ origin: "https://evil.example" }));
    await expect(invoke(setAgentEnabled, app, { id: "agent", enabled: false })).rejects.toMatchObject({ status: 403 });
    await expect(invoke(closeAgentPosition, app, { id: "agent", symbol: "BTC", requestId: "close-2" })).rejects.toMatchObject({ status: 403 });
    expect(closePosition).toHaveBeenCalledTimes(1); expect(setEnabled).toHaveBeenCalledTimes(1);
    mocks.user.mockResolvedValue(null);
    mocks.headers.mockReturnValue(new Headers({ origin: runtime.origin }));
    expect(isRedirect(await invoke(setAgentEnabled, app, { id: "agent", enabled: false }).catch((error: unknown) => error))).toBe(true);
  });

  it("checks cancellation ownership and returns the operator result with the original request ID", async () => {
    const owned = vi.fn().mockRejectedValue(new AgentError("Agent not found", 404));
    const cancelOrder = vi.fn();
    const app = { ...runtime, agents: { owned }, orders: { cancelOrder } } as unknown as AppRuntime;
    const input = { id: "agent", symbol: "BTC", oid: "123", requestId: "cancel-1" };
    await expect(invoke(cancelAgentOrder, app, input)).rejects.toMatchObject({ status: 404 });
    expect(owned).toHaveBeenCalledWith("owner", "agent");
    expect(cancelOrder).not.toHaveBeenCalled();
    owned.mockResolvedValue({ id: "agent", enabled: false });
    const result = { operationId: "operation", requestId: "cancel-1", kind: "cancel", results: [], uncertain: false, warnings: [] };
    cancelOrder.mockResolvedValue(result);
    for (let retry = 0; retry < 2; retry++) {
      expect(await invoke(cancelAgentOrder, app, { ...input, account: "foreign", operator: false })).toEqual(result);
    }
    expect(cancelOrder).toHaveBeenCalledTimes(2);
    expect(cancelOrder).toHaveBeenLastCalledWith("owner", "agent", "BTC", "123", "cancel-1");
    const conflict = new HyperliquidError("Cancellation target was not found", 409);
    cancelOrder.mockRejectedValue(conflict);
    await expect(invoke(cancelAgentOrder, app, input)).rejects.toBe(conflict);
    cancelOrder.mockRejectedValue(new Error("private signing details"));
    await expect(invoke(cancelAgentOrder, app, input)).rejects.toMatchObject({ message: "Agent operation failed. Please try again.", status: 500 });
  });

  it("rejects invalid cancellation fields, cross-origin requests, and unauthenticated callers before service calls", async () => {
    const owned = vi.fn(), cancelOrder = vi.fn();
    const app = { ...runtime, agents: { owned }, orders: { cancelOrder } } as unknown as AppRuntime;
    const input = { id: "agent", symbol: "BTC", oid: "123", requestId: "cancel-1" };
    for (const field of ["id", "symbol", "oid", "requestId"]) {
      expect(() => invoke(cancelAgentOrder, app, { ...input, [field]: 123 })).toThrow("Invalid request");
      expect(() => invoke(cancelAgentOrder, app, { ...input, [field]: undefined })).toThrow("Invalid request");
    }
    for (const headers of [new Headers({ origin: "https://evil.example" }), new Headers()]) {
      mocks.headers.mockReturnValue(headers);
      await expect(invoke(cancelAgentOrder, app, input)).rejects.toMatchObject({ status: 403 });
    }
    mocks.headers.mockReturnValue(new Headers({ origin: runtime.origin }));
    mocks.user.mockResolvedValue(null);
    expect(isRedirect(await invoke(cancelAgentOrder, app, input).catch((error: unknown) => error))).toBe(true);
    expect(owned).not.toHaveBeenCalled();
    expect(cancelOrder).not.toHaveBeenCalled();
  });

  it("keeps authentication redirects out of the service boundary", async () => {
    mocks.user.mockResolvedValue(null);
    const result: unknown = await invoke(getProviderSettings, runtime).catch((error: unknown) => error);
    expect(isRedirect(result)).toBe(true); expect(settings).not.toHaveBeenCalled();
  });
});

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
import { getNotification, getNotificationSettings, listNotifications, saveNotificationSettings } from "./notifications.js";
function invoke(fn: unknown, runtime: AppRuntime, input: unknown) { return (fn as { invoke(runtime: AppRuntime, input: unknown): Promise<unknown> }).invoke(runtime, input); }
describe("authenticated Agent notifications", () => {
  const owned = vi.fn(), list = vi.fn(), get = vi.fn(), getSettings = vi.fn(), saveSettings = vi.fn();
  let runtime: AppRuntime;
  beforeEach(() => {
    vi.resetAllMocks(); mocks.currentUser.mockResolvedValue({ id: "owner" }); mocks.headers.mockReturnValue(new Headers({ origin: "http://localhost:3003" }));
    owned.mockImplementation(async (_user: string, id: string) => { if (id !== "agent") throw new AgentError("Agent not found", 404); });
    runtime = { origin: "http://localhost:3003", status: vi.fn(), agents: { owned }, notifications: { list, get, getSettings, saveSettings } } as unknown as AppRuntime;
  });
  it("checks ownership before history, details, settings or mutations", async () => {
    for (const fn of [listNotifications, getNotification, getNotificationSettings, saveNotificationSettings]) {
      await expect(invoke(fn, runtime, { agentId: "foreign", id: "n1", enabled: true, submissions: true, fills: true })).rejects.toMatchObject({ status: 404 });
    }
    for (const service of [list, get, getSettings, saveSettings]) expect(service).not.toHaveBeenCalled();
    await invoke(listNotifications, runtime, { agentId: "agent", page: 2 }); expect(list).toHaveBeenCalledWith("agent", 2);
    await invoke(getNotification, runtime, { agentId: "agent", id: "n1" }); expect(get).toHaveBeenCalledWith("agent", "n1");
  });
  it("rejects unauthenticated requests and cross-origin writes", async () => {
    mocks.currentUser.mockResolvedValue(null);
    await expect(invoke(listNotifications, runtime, { agentId: "agent" })).rejects.toBeDefined();
    expect(list).not.toHaveBeenCalled();
    mocks.currentUser.mockResolvedValue({ id: "owner" }); mocks.headers.mockReturnValue(new Headers({ origin: "https://evil.test" }));
    await expect(invoke(saveNotificationSettings, runtime, { agentId: "agent", enabled: true, submissions: true, fills: true })).rejects.toMatchObject({ status: 403 });
    expect(saveSettings).not.toHaveBeenCalled();
  });
  it("validates booleans and sanitizes unexpected service errors", async () => {
    expect(() => invoke(saveNotificationSettings, runtime, { agentId: "agent", enabled: "false", submissions: true, fills: true })).toThrow("Invalid request");
    list.mockRejectedValue(new Error("secret database detail"));
    await expect(invoke(listNotifications, runtime, { agentId: "agent" })).rejects.toMatchObject({ message: "Agent operation failed. Please try again.", status: 500 });
  });
});

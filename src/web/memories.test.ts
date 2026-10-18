import { beforeEach, describe, expect, it, vi } from "vitest";
import { randomUUID } from "node:crypto";
import type { AppRuntime } from "../runtime.js";
import { AgentError } from "../agents/agent-input.js";
import { LearningConflict } from "../memory/learning-service.js";
const mocks = vi.hoisted(() => ({ currentUser: vi.fn(), headers: vi.fn() }));
vi.mock("../auth-user.js", () => ({ currentUser: mocks.currentUser }));
vi.mock("@tanstack/react-start/server", () => ({ getRequestHeaders: mocks.headers }));
// Exercise the actual validators/handlers without a listener or built SSR host.
vi.mock("@tanstack/react-start", () => {
  const builder = (validate: (data: unknown) => unknown = (data) => data) => ({
    validator: (next: (data: unknown) => unknown) => builder(next),
    handler: (handler: (args: { context: { runtime: AppRuntime }; data: unknown }) => Promise<unknown>) => ({ invoke: (runtime: AppRuntime, input: unknown) => handler({ context: { runtime }, data: validate(input) }) }),
  });
  return { createServerFn: () => builder() };
});
import { getLearnings, getMemory, listMemories, replaceLearnings } from "./memories.js";
async function invoke(fn: unknown, runtime: AppRuntime, input: unknown): Promise<unknown> { return (fn as { invoke(runtime: AppRuntime, input: unknown): Promise<unknown> }).invoke(runtime, input); }
describe("authenticated Agent-owned memory server functions", () => {
  let runtime: AppRuntime;
  const owned = vi.fn(), list = vi.fn(), get = vi.fn(), current = vi.fn(), revision = vi.fn(), history = vi.fn(), replace = vi.fn(), operator = vi.fn();
  beforeEach(() => {
    vi.resetAllMocks(); mocks.currentUser.mockResolvedValue({ id: "owner" }); mocks.headers.mockReturnValue(new Headers({ origin: "http://localhost:3003" }));
    owned.mockImplementation(async (user: string, id: string) => { if (user !== "owner" || id !== "agent") throw new AgentError("Agent not found", 404); });
    list.mockResolvedValue({ items: [], nextCursor: null }); get.mockResolvedValue({ ref: "m1" });
    current.mockResolvedValue({ scope: { kind: "agent" }, version: 0, content: "" }); history.mockResolvedValue({ items: [], nextBeforeVersion: null });
    operator.mockReturnValue({ agentId: "agent", userId: "owner" });
    runtime = { origin: "http://localhost:3003", status: vi.fn(), agents: { owned, execution: { memories: { list, get }, learnings: { current, revision, history, replace }, operator } } } as unknown as AppRuntime;
  });
  it("checks ownership before every read and forwards bounded query contracts to the service", async () => {
    expect(await invoke(listMemories, runtime, { agentId: "agent", filters: { kind: "research", limit: 25 } })).toEqual({ ok: true, data: { items: [], nextCursor: null } });
    expect(list).toHaveBeenCalledWith("agent", { kind: "research", limit: 25 });
    expect(await invoke(getMemory, runtime, { agentId: "foreign", ref: "m1" })).toMatchObject({ ok: false, error: "Agent not found" }); expect(get).not.toHaveBeenCalled();
    await invoke(getLearnings, runtime, { agentId: "agent", scope: { kind: "agent" }, history: true });
    expect(current).toHaveBeenCalledWith("agent", { kind: "agent" }); expect(history).toHaveBeenCalledWith("agent", { kind: "agent" }, {});
    await expect(invoke(listMemories, runtime, { agentId: "agent", unknownGlobalScope: true })).rejects.toThrow();
    await expect(invoke(getLearnings, runtime, { agentId: "agent", scope: { kind: "agent" }, history: "true" })).rejects.toThrow("history flag");
  });
  it("requires fresh authentication and same-origin mutation, with no effects on denial", async () => {
    mocks.headers.mockReturnValue(new Headers({ origin: "https://evil.example" }));
    await expect(invoke(replaceLearnings, runtime, { agentId: "agent", requestId: randomUUID(), input: {} })).rejects.toMatchObject({ status: 403 });
    expect(mocks.currentUser).not.toHaveBeenCalled(); expect(replace).not.toHaveBeenCalled();
    mocks.headers.mockReturnValue(new Headers({ origin: runtime.origin })); mocks.currentUser.mockResolvedValue(null);
    await expect(invoke(getMemory, runtime, { agentId: "agent", ref: "m1" })).rejects.toBeDefined(); expect(get).not.toHaveBeenCalled();
  });
  it("uses authenticated operator provenance and stable UI receipt keys, preserving visible CAS conflicts", async () => {
    const id = randomUUID(), input = { scope: { kind: "agent" }, expectedVersion: 1, content: "Draft", reason: "Consolidate", evidenceRefs: [] };
    replace.mockRejectedValue(new LearningConflict(2));
    expect(await invoke(replaceLearnings, runtime, { agentId: "agent", requestId: id, input })).toMatchObject({ ok: false, error: expect.stringContaining("v2") });
    expect(operator).toHaveBeenCalledWith("agent", "owner"); expect(replace).toHaveBeenCalledWith({ agentId: "agent", userId: "owner" }, `ui:${id}`, input);
    await expect(invoke(replaceLearnings, runtime, { agentId: "agent", requestId: "model-supplied-task", input })).rejects.toThrow("request ID");
  });
});

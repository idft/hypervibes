import { isNotFound } from "@tanstack/react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AgentError } from "../agents/agent-input.js";
import type { AppRuntime } from "../runtime.js";

vi.mock("@tanstack/react-start", () => ({
  createServerFn: () => ({
    validator: () => ({ handler: (handler: unknown) => handler }),
    handler: (handler: unknown) => handler,
  }),
}));
vi.mock("@tanstack/react-start/server", () => ({ getRequestHeaders: () => new Headers() }));
vi.mock("../auth-user.js", () => ({ currentUser: async () => ({ id: "owner" }) }));
import { getAgent } from "./agents.js";

const loadAgent = getAgent as unknown as (input: {
  context: { runtime: AppRuntime };
  data: { id: string };
}) => Promise<unknown>;

describe("agent page loading", () => {
  const owned = vi.fn();
  const runtime = { status: vi.fn(), agents: { owned } } as unknown as AppRuntime;
  const load = () => loadAgent({ context: { runtime }, data: { id: "old-agent" } });

  beforeEach(() => vi.resetAllMocks());

  it("returns the owned agent for the route layout", async () => {
    const agent = { id: "old-agent", name: "Agent" };
    owned.mockResolvedValue(agent);
    expect(await load()).toEqual(agent);
    expect(owned).toHaveBeenCalledWith("owner", "old-agent", true);
  });

  it("signals a router 404 for missing or inaccessible agents", async () => {
    owned.mockRejectedValue(new AgentError("Agent not found", 404));
    expect(isNotFound(await load().catch((error: unknown) => error))).toBe(true);
  });

  it("propagates other failures to the styled error boundary rather than treating them as missing agents", async () => {
    const error = new Error("Database unavailable");
    owned.mockRejectedValue(error);
    await expect(load()).rejects.toBe(error);
  });
});

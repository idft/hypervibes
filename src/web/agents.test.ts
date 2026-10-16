import { isRedirect } from "@tanstack/react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AgentError } from "../agents/agent-input.js";
import type { AppRuntime } from "../runtime.js";

const mocks = vi.hoisted(() => ({ currentUser: vi.fn() }));
vi.mock("../auth-user.js", () => ({ currentUser: mocks.currentUser }));
import { agentAction, agentUser, selectedTradingAccount } from "./agents.js";

const runtime = { origin: "http://localhost:3003", status: vi.fn() } as unknown as AppRuntime;
describe("Agent server-function boundary", () => {
  beforeEach(() => vi.resetAllMocks());
  it("requires a fresh authenticated identity for every operation", async () => {
    const headers = new Headers();
    mocks.currentUser.mockResolvedValue({ id: "owner", walletAddress: "0xfixture" });
    expect(await agentUser(runtime, headers)).toMatchObject({ id: "owner" });
    expect(mocks.currentUser).toHaveBeenCalledWith(runtime, headers);
    mocks.currentUser.mockResolvedValue(null);
    expect(isRedirect(await agentUser(runtime, headers).catch((error: unknown) => error))).toBe(true);
  });
  it.each(["https://evil.example", "null", ""])("rejects a mutation from an invalid origin: %s", async (origin) => {
    await expect(agentUser(runtime, new Headers({ origin }), true)).rejects.toMatchObject({ status: 403 });
    expect(mocks.currentUser).not.toHaveBeenCalled();
  });
  it("allows authenticated same-origin mutations but propagates auth/storage failure", async () => {
    mocks.currentUser.mockResolvedValue({ id: "owner" });
    expect(await agentUser(runtime, new Headers({ origin: runtime.origin }), true)).toMatchObject({ id: "owner" });
    mocks.currentUser.mockRejectedValue(new Error("storage unavailable"));
    await expect(agentUser(runtime, new Headers())).rejects.toThrow("storage unavailable");
  });
  it("rejects account-less creation and fails closed on unavailable account choices", async () => {
    const choices = vi.fn().mockResolvedValue({ lookupError: "Exchange unavailable", accountModeSupported: true, apiWalletReady: true, mainAddress: "0xfixture", mainAssignedTo: null });
    const app = { hyperliquid: { tradingAccountChoices: choices } } as unknown as AppRuntime;
    const user = { id: "owner", walletAddress: "0xfixture" };
    await expect(selectedTradingAccount(app, user, "none")).rejects.toThrow("Select an available");
    expect(choices).not.toHaveBeenCalled();
    await expect(selectedTradingAccount(app, user, "main")).rejects.toThrow("Exchange unavailable");
  });
  it("returns public errors without disclosing unexpected provider/storage errors", async () => {
    expect(await agentAction(async () => { throw new AgentError("Agent not found", 404); })).toEqual({ ok: false, error: "Agent not found" });
    const result = await agentAction(async () => { throw new Error("secret-key-fixture sql stack"); });
    expect(result).toEqual({ ok: false, error: "Agent operation failed. Please try again." });
    expect(JSON.stringify(result)).not.toContain("secret-key");
  });
});

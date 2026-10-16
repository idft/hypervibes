import { isRedirect } from "@tanstack/react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ user: vi.fn(), destination: vi.fn() }));
vi.mock("./auth.js", () => ({ getCurrentUser: mocks.user }));
vi.mock("./providers.js", () => ({ getLoginDestination: mocks.destination }));
import { Route as ProtectedRoute } from "../routes/_authenticated.js";
import { Route as LoginRoute } from "../routes/login.js";

// These beforeLoad guards don't take route parameters; invoke the actual hooks.
const protectedGuard = ProtectedRoute.options.beforeLoad as () => Promise<unknown>;
const loginGuard = LoginRoute.options.beforeLoad as () => Promise<unknown>;

describe("auth route guards", () => {
  beforeEach(() => vi.resetAllMocks());

  it("redirects anonymous private navigation to login", async () => {
    mocks.user.mockResolvedValue(null);
    const result = await protectedGuard().catch((error: unknown) => error);
    expect(isRedirect(result)).toBe(true);
    if (isRedirect(result)) expect(result.options.to).toBe("/login");
  });

  it("supplies public identity to the authenticated layout", async () => {
    const user = { id: "fixture", walletAddress: `0x${"11".repeat(20)}` };
    mocks.user.mockResolvedValue(user);
    expect(await protectedGuard()).toEqual({ user });
  });

  it.each(["/providers", "/agents"])("redirects an authenticated login-page visit to %s", async (destination) => {
    mocks.user.mockResolvedValue({ id: "fixture", walletAddress: `0x${"11".repeat(20)}` });
    mocks.destination.mockResolvedValue(destination);
    const result = await loginGuard().catch((error: unknown) => error);
    expect(isRedirect(result)).toBe(true);
    if (isRedirect(result)) expect(result.options.to).toBe(destination);
  });

  it("keeps login public and propagates auth failures instead of granting access", async () => {
    mocks.user.mockResolvedValue(null);
    await expect(loginGuard()).resolves.toBeUndefined();
    const failure = new Error("Database unavailable");
    mocks.user.mockRejectedValue(failure);
    await expect(protectedGuard()).rejects.toBe(failure);
  });
});

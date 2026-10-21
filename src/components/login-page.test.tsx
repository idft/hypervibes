// @vitest-environment jsdom
import { act } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createRoot } from "react-dom/client";
import type { Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ login: vi.fn(), destination: vi.fn() }));
vi.mock("../web/providers.js", () => ({ getLoginDestination: mocks.destination }));
vi.mock("../web/wallet-login.js", async (original) => ({
  ...await original<typeof import("../web/wallet-login.js")>(), walletLogin: mocks.login,
}));
import { LoginPage } from "./login-page.js";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("login page", () => {
  let container: HTMLDivElement;
  let root: Root;
  let client: QueryClient;
  beforeEach(async () => {
    vi.resetAllMocks();
    client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity } } });
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    await act(async () => root.render(<QueryClientProvider client={client}><LoginPage /></QueryClientProvider>));
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
    vi.restoreAllMocks();
  });

  it("renders the existing centered login card without authenticated navigation", () => {
    expect(container.querySelector("h1")?.textContent).toBe("HyperVibes");
    expect(container.querySelector("button")?.textContent).toBe("Login");
    expect(container.querySelector("button")?.className).toContain("cursor-pointer");
    expect(container.querySelector("button")?.className).toContain("border-violet-500");
    expect(container.querySelector("button")?.className).not.toContain("bg-violet-600");
    expect(container.querySelector("nav")).toBeNull();
    expect(container.querySelector("[role=alert]")?.textContent).toBe("");
  });

  it.each(["/providers", "/agents"])("navigates to %s after wallet login according to provider readiness", async (destination) => {
    mocks.login.mockResolvedValue(undefined);
    mocks.destination.mockResolvedValue(destination);
    const assign = vi.fn();
    const location = window.location;
    vi.spyOn(window, "location", "get").mockReturnValue({ ...location, assign } as unknown as Location);
    await act(async () => container.querySelector("button")!.click());
    expect(assign).toHaveBeenCalledWith(destination);
  });

  it("prevents duplicate submissions, announces progress and permits retry after cancellation", async () => {
    let reject: (reason: unknown) => void = () => {};
    mocks.login.mockImplementation(() => new Promise<void>((_resolve, fail) => { reject = fail; }));
    const button = container.querySelector("button")!;
    await act(async () => {
      button.click();
      button.click();
    });
    expect(mocks.login).toHaveBeenCalledOnce();
    expect(button.disabled).toBe(true);
    await act(async () => mocks.login.mock.calls[0]![0].onStatus("Sign message in your wallet…"));
    expect(button.textContent).toBe("Sign message in your wallet…");
    expect(button.querySelector("[role=status]")?.textContent).toBe("Sign message in your wallet…");
    await act(async () => reject({ code: 4001 }));
    expect(button.disabled).toBe(false);
    expect(container.querySelector("[role=alert]")?.textContent).toContain("Sign-in cancelled");
    expect(button.textContent).toBe("Login");
    await act(async () => button.click());
    expect(mocks.login).toHaveBeenCalledTimes(2);
    expect(container.querySelector("[role=alert]")?.textContent).toBe("");
    await act(async () => reject(new Error("No Ethereum wallet found.")));
    expect(container.querySelector("[role=alert]")?.textContent).toBe("No Ethereum wallet found.");
  });
});

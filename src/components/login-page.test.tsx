// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import type { Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ login: vi.fn() }));
vi.mock("../web/wallet-login.js", async (original) => ({
  ...await original<typeof import("../web/wallet-login.js")>(), walletLogin: mocks.login,
}));
import { LoginPage } from "./login-page.js";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("login page", () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(async () => {
    vi.resetAllMocks();
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    await act(async () => root.render(<LoginPage />));
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it("renders the existing centered login card without authenticated navigation", () => {
    expect(container.querySelector("h1")?.textContent).toBe("HyperVibes");
    expect(container.querySelector("button")?.textContent).toBe("Login");
    expect(container.querySelector("button")?.className).toContain("cursor-pointer");
    expect(container.querySelector("nav")).toBeNull();
    expect(container.querySelector("[role=alert]")?.textContent).toBe("");
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
    await act(async () => mocks.login.mock.calls[0]![0].onStatus("Confirm in wallet"));
    expect(container.querySelector("[role=status]")?.textContent).toBe("Confirm in wallet");
    await act(async () => reject({ code: 4001 }));
    expect(button.disabled).toBe(false);
    expect(container.querySelector("[role=alert]")?.textContent).toContain("Sign-in cancelled");
    expect(container.querySelector("[role=status]")?.textContent).toBe("");
    await act(async () => button.click());
    expect(mocks.login).toHaveBeenCalledTimes(2);
    expect(container.querySelector("[role=alert]")?.textContent).toBe("");
    await act(async () => reject(new Error("No Ethereum wallet found.")));
    expect(container.querySelector("[role=alert]")?.textContent).toBe("No Ethereum wallet found.");
  });
});

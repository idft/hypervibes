// @vitest-environment jsdom
import { act } from "react";
import { type QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createTestQueryClient } from "../test/query.js";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ModelChoice } from "../providers/provider-service.js";
const mocks = vi.hoisted(() => ({ create: vi.fn(), invalidate: vi.fn() }));
vi.mock("@tanstack/react-router", () => ({ useRouter: () => ({ invalidate: mocks.invalidate }) }));
import { CreateAgentPage } from "./create-agent-page.js";
const model: ModelChoice = { provider: "test", modelId: "model", name: "Model", providerName: "Test", thinkingLevels: ["low", "high"], contextWindow: 1000, inputCost: 0, outputCost: 0, reasoning: true };

describe("Agent creation default sub-agents", () => {
  let root: Root, container: HTMLDivElement;
  let queryClient: QueryClient;
  const submit = () => act(async () => { container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
  beforeEach(async () => {
    vi.resetAllMocks();
    queryClient = createTestQueryClient();
    mocks.create.mockRejectedValue(new Error("Retry safely"));
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(<QueryClientProvider client={queryClient}><CreateAgentPage choices={{ mainAddress: `0x${"a".repeat(40)}`, mainBalance: null, mainAssignedTo: null, subaccounts: [], subaccountCapacity: null, lookupError: null, accountModeSupported: true, apiWalletReady: true }} models={[model]} create={mocks.create} createSubaccount={vi.fn()} /></QueryClientProvider>));
    await act(async () => {
      const input = container.querySelector<HTMLInputElement>("#display_name")!;
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "Desk");
      input.dispatchEvent(new Event("input", { bubbles: true }));
      [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("Choose model"))!.click();
    });
    await act(async () => document.querySelector<HTMLInputElement>('dialog input[type="radio"]')!.click());
    await act(async () => [...document.querySelectorAll<HTMLButtonElement>("dialog button")].find((button) => button.textContent === "Apply")!.click());
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
  it("creates over HTTP without crypto.randomUUID and keeps the same request ID for retries", async () => {
    const getRandomValues = vi.fn(globalThis.crypto.getRandomValues.bind(globalThis.crypto));
    vi.stubGlobal("crypto", { getRandomValues });
    await submit();
    expect(getRandomValues).toHaveBeenCalled();
    expect(mocks.create).toHaveBeenCalledWith("Desk", expect.stringMatching(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/), "main", model, true, "");
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Retry safely");
    await submit();
    expect(mocks.create.mock.calls[1]).toEqual(mocks.create.mock.calls[0]);
    expect(getRandomValues).toHaveBeenCalledTimes(1);
  });
  it("shows errors before the mutation starts and allows another attempt", async () => {
    const crypto = globalThis.crypto;
    vi.stubGlobal("crypto", { getRandomValues: () => { throw new Error("Randomness unavailable"); } });
    await submit();
    expect(mocks.create).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Randomness unavailable");
    vi.stubGlobal("crypto", { getRandomValues: crypto.getRandomValues.bind(crypto) });
    await submit();
    expect(mocks.create).toHaveBeenCalledTimes(1);
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Retry safely");
    expect(container.textContent).not.toContain("Randomness unavailable");
  });
  it("includes paused defaults by default, passes the selection and reuses the creation ID on retry", async () => {
    expect(container.textContent).toContain("Include default sub-agents");
    expect(container.textContent).toContain("Adds a connected Technical Analyst, Trader, and Daily Reviewer. All start paused.");
    expect(container.querySelector<HTMLInputElement>('input[type="checkbox"]')!.checked).toBe(true);
    await submit();
    expect(mocks.create).toHaveBeenCalledWith("Desk", expect.any(String), "main", model, true, "");
    await submit();
    expect(mocks.create.mock.calls[1]).toEqual(mocks.create.mock.calls[0]);
  });
  it("shows thinking controls, preserves the selection and submits it with a fresh request ID", async () => {
    await submit();
    const firstRequest = mocks.create.mock.calls[0]![1];
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("Choose model"))!.click());
    const thinking = document.querySelector<HTMLSelectElement>("dialog select")!;
    expect(thinking).not.toBeNull();
    await act(async () => { thinking.value = "high"; thinking.dispatchEvent(new Event("change", { bubbles: true })); });
    await act(async () => [...document.querySelectorAll<HTMLButtonElement>("dialog button")].find((button) => button.textContent === "Apply")!.click());
    expect(container.textContent).toContain("Thinking: high");
    await submit();
    expect(mocks.create).toHaveBeenLastCalledWith("Desk", expect.any(String), "main", model, true, "high");
    expect(mocks.create.mock.calls[1]![1]).not.toBe(firstRequest);
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("Choose model"))!.click());
    expect(document.querySelector<HTMLSelectElement>("dialog select")!.value).toBe("high");
  });
  it("allows opting out and gives changed settings a fresh creation request ID", async () => {
    await submit();
    await act(async () => container.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
    await submit();
    expect(mocks.create).toHaveBeenLastCalledWith("Desk", expect.any(String), "main", model, false, "");
    expect(mocks.create.mock.calls[1]![1]).not.toBe(mocks.create.mock.calls[0]![1]);
  });
});

// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClientProvider, type QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createTestQueryClient } from "../test/query.js";
import type { OperationResult, OrderView } from "../hyperliquid/orders/order-contracts.js";
const mocks = vi.hoisted(() => ({ cancel: vi.fn() }));
vi.mock("../web/agents.js", () => ({ cancelAgentOrder: mocks.cancel }));
import { CancelOrderButton } from "./cancel-order-button.js";

const outcome = (uncertain = false): OperationResult => ({ operationId: "operation", requestId: "request", kind: "cancel", uncertain, warnings: [], results: [{ id: "order", orderStatus: uncertain ? "open" : "canceled", error: null } as OrderView] });
describe("Cancel order button", () => {
  let container: HTMLDivElement;
  let root: Root;
  let client: QueryClient;
  const button = () => container.querySelector("button")!;
  const click = async () => act(async () => button().click());
  beforeEach(async () => {
    mocks.cancel.mockReset();
    client = createTestQueryClient();
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(<QueryClientProvider client={client}><CancelOrderButton agentId="agent" symbol="BTC" oid="123" /></QueryClientProvider>));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); client.clear(); vi.useRealTimers(); });
  it("requires confirmation, cancels only the selected order, and reports success", async () => {
    mocks.cancel.mockResolvedValue(outcome());
    expect(button().getAttribute("aria-label")).toBe("Cancel BTC order 123");
    expect(button().classList.contains("cursor-pointer")).toBe(true);
    await click();
    expect(mocks.cancel).not.toHaveBeenCalled();
    expect(button().textContent).toContain("Confirm cancel");
    const invalidation = vi.spyOn(client, "invalidateQueries");
    await click();
    expect(mocks.cancel).toHaveBeenCalledOnce();
    expect(mocks.cancel.mock.calls[0]![0]).toEqual({ data: { id: "agent", symbol: "BTC", oid: "123", requestId: expect.any(String) } });
    expect(invalidation).toHaveBeenCalledWith({ queryKey: ["agent-dashboard", "agent"] });
    expect(container.querySelector('[role="status"]')?.textContent).toBe("Order canceled.");
    expect(button().disabled).toBe(true);
  });
  it("dismisses confirmation with Escape or after 8 seconds", async () => {
    vi.useFakeTimers();
    await click();
    await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    expect(button().textContent).not.toContain("Confirm cancel");
    expect(document.activeElement).toBe(button());
    await click();
    await act(async () => vi.advanceTimersByTime(8_000));
    expect(button().textContent).not.toContain("Confirm cancel");
    expect(mocks.cancel).not.toHaveBeenCalled();
  });
  it("blocks duplicate requests and keeps the request ID after a lost response and uncertain result", async () => {
    let reject!: (error: Error) => void;
    mocks.cancel.mockImplementationOnce(() => new Promise((_resolve, fail) => { reject = fail; }));
    await click();
    await act(async () => { button().click(); button().click(); });
    expect(mocks.cancel).toHaveBeenCalledOnce();
    expect(button().disabled).toBe(true);
    const first = mocks.cancel.mock.calls[0]![0];
    await act(async () => reject(new Error("Connection lost")));
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Retry uses the same request ID");
    mocks.cancel.mockResolvedValueOnce(outcome(true));
    await click(); await click();
    expect(mocks.cancel.mock.calls[1]![0]).toEqual(first);
    expect(container.querySelector('[role="status"]')?.textContent).toContain("unconfirmed");
    expect(button().disabled).toBe(false);
    mocks.cancel.mockResolvedValueOnce(outcome());
    await click(); await click();
    expect(mocks.cancel.mock.calls[2]![0]).toEqual(first);
    expect(container.querySelector('[role="status"]')?.textContent).toBe("Order canceled.");
  });
});

// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { JournalPage } from "../hyperliquid/journal-service.js";

const mocks = vi.hoisted(() => ({ getAgentJournal: vi.fn(), getJournalNotes: vi.fn(), addJournalNote: vi.fn() }));
vi.mock("../web/journal.js", () => mocks);
import { AgentJournalPage } from "./agent-journal-page.js";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const initial: JournalPage = {
  linked: true, view: "activity", page: 1, pages: 2, total: 26, pending: false, historyLimited: false, trades: [],
  activity: [{ id: "deposit", kind: "ledger", time: 1000, symbol: "USDC", type: "deposit", delta: "123.4567", fee: null, pnl: null, fill: null, balance: "123.4567", noteCount: 0 }],
  sync: ["fill", "funding", "ledger"].map((stream) => ({ stream, cursor: 1000, updated_at: 1000, error: null })),
};
describe("Agent journal UI", () => {
  let root: Root; let container: HTMLDivElement;
  const navigate = vi.fn();
  beforeEach(() => {
    vi.resetAllMocks();
    Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value: function (this: HTMLDialogElement) { this.open = true; } });
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    mocks.getAgentJournal.mockResolvedValue({ ok: true, data: initial });
    mocks.getJournalNotes.mockResolvedValue({ ok: true, data: [] });
    mocks.addJournalNote.mockResolvedValue({ ok: true });
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.useRealTimers(); });
  const render = async (data = initial) => { await act(async () => root.render(<AgentJournalPage id="agent" initial={data} onNavigate={navigate} />)); };
  const click = async (text: string) => { await act(async () => [...container.querySelectorAll("button")].find((b) => b.textContent === text)!.click()); };

  it("matches the legacy tabs, activity columns and pagination with exact formatted amounts", async () => {
    await render();
    expect(container.textContent).toContain("Transactions");
    expect([...container.querySelectorAll("th")].map((th) => th.textContent).slice(0, 3)).toEqual(["Time", "Notes", "Type"]);
    expect(container.querySelectorAll("tbody td")[1]?.querySelector("button")?.getAttribute("aria-label")).toBe("Add transaction notes");
    expect(container.textContent).toContain("123.4567");
    expect(container.textContent).toContain("10,000 historical fills");
    expect(container.querySelector("button")?.disabled).toBe(true);
    await click("Next"); expect(navigate).toHaveBeenLastCalledWith("activity", 2);
    await click("Trades"); expect(navigate).toHaveBeenLastCalledWith("trades", 1);
    expect(mocks.getAgentJournal).not.toHaveBeenCalled();
  });
  it("labels exchange fill sides as Sell and Buy without changing source history", async () => {
    await render({ ...initial, activity: [
      { ...initial.activity[0]!, id: "sell", kind: "fill", type: "fill A" },
      { ...initial.activity[0]!, id: "buy", kind: "fill", type: "fill B" },
    ] });
    expect([...container.querySelectorAll("tbody tr")].map((row) => row.querySelectorAll("td")[2]?.textContent)).toEqual(["Fill · Sell", "Fill · Buy"]);
  });
  it("shows perpetual funding logos in Activity, leaving spot and ledger assets unchanged", async () => {
    await render({ ...initial, activity: [
      { ...initial.activity[0]!, id: "funding", kind: "funding", symbol: "BTC", type: "funding" },
      { ...initial.activity[0]!, id: "spot", kind: "fill", symbol: "@107", type: "fill B" },
      initial.activity[0]!,
    ] });
    expect([...container.querySelectorAll("tbody img")].map((image) => image.getAttribute("src"))).toEqual(["/currency/BTC.svg"]);
    expect(container.textContent).toContain("@107");
    expect(container.textContent).toContain("USDC");
  });
  it("opens notes, renders authored text safely and saves a scoped note", async () => {
    await render();
    mocks.getJournalNotes.mockResolvedValue({ ok: true, data: [{ id: "n", body: "<script>not HTML</script>", authorId: "owner", createdAt: 1000 }] });
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Add transaction notes"]')!.click());
    expect(container.querySelector("dialog")?.open).toBe(true);
    expect(container.querySelector("script")).toBeNull();
    expect(container.textContent).toContain("<script>not HTML</script>");
    const textarea = container.querySelector("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, "My reasoning");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(mocks.addJournalNote).toHaveBeenCalledWith({ data: { id: "agent", target: "deposit", body: "My reasoning" } });
    expect(textarea.value).toBe("");
    await click("Close"); expect(container.querySelector("dialog")).toBeNull();
  });
  it("preserves note drafts on rejection and displays incomplete history distinctly", async () => {
    await render({ ...initial, pending: true, activity: [{ ...initial.activity[0]!, balance: null }], sync: [{ ...initial.sync[0]!, error: "History sync failed" }] });
    expect(container.textContent).toContain("Synchronizing account history");
    expect(container.textContent).toContain("Incomplete");
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("failed");
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Add transaction notes"]')!.click());
    mocks.addJournalNote.mockResolvedValue({ ok: false, error: "Not authorized" });
    const textarea = container.querySelector("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, "Keep my draft");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(textarea.value).toBe("Keep my draft");
    expect(container.querySelector("dialog")?.textContent).toContain("Not authorized");
  });
  it("shows grouped trades, coverage warnings and the constituent fills in the notes modal", async () => {
    await render({ ...initial, view: "trades", activity: [], trades: [{
      id: "trade", symbol: "BTC", direction: "long", status: "incomplete", openedAt: 1000, lastActivityAt: 2000, closedAt: null,
      remainingSize: "1", entrySize: "2", exitSize: "1", entryNotional: "200.24691357", exitNotional: "120.98765432", grossPnl: "20", fees: "0.1", funding: null, net: null,
      feeCoverage: false, coverageReason: "Fee is missing", noteCount: 1,
      parts: [{ eventId: "fill-1", time: 1000, price: "100.123456789", side: "B", segment: "entry", quantity: "2", fee: "0.1", pnl: "0" }],
    }] });
    expect(container.textContent).toContain("BTC · long");
    expect(container.querySelector("tbody img")?.getAttribute("src")).toBe("/currency/BTC.svg");
    expect(container.textContent).toContain("Incomplete coverage");
    expect([...container.querySelectorAll("th")].map((th) => th.textContent).slice(0, 4)).toEqual(["Trade", "Status", "Notes", "Last activity"]);
    expect(container.querySelectorAll("tbody td")[2]?.querySelector("button")?.getAttribute("aria-label")).toBe("View BTC trade notes");
    expect(container.textContent).toContain("100.1235 / 120.9877");
    expect(container.textContent).not.toContain("100.123456");
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="View BTC trade notes"]')!.click());
    expect(container.querySelector("summary")?.textContent).toBe("Trade fills");
    expect(container.querySelector("details")?.textContent).toContain("entry 2 @ 100.1235");
  });
  it("shows an unlinked account without starting requests", async () => {
    vi.useFakeTimers();
    await render({ ...initial, linked: false, activity: [], total: 0 });
    await act(async () => vi.advanceTimersByTime(10_000));
    expect(container.textContent).toContain("No trading account");
    expect(mocks.getAgentJournal).not.toHaveBeenCalled();
  });
  it("refreshes while mounted and stops polling on unmount", async () => {
    vi.useFakeTimers(); await render();
    await act(async () => vi.advanceTimersByTimeAsync(5_000));
    expect(mocks.getAgentJournal).toHaveBeenCalledOnce();
    await act(async () => root.render(null));
    await act(async () => vi.advanceTimersByTimeAsync(10_000));
    expect(mocks.getAgentJournal).toHaveBeenCalledOnce();
  });
});

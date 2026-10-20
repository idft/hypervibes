// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ instruments: vi.fn(), change: vi.fn() }));
vi.mock("../web/charts.js", () => ({ getChartInstruments: mocks.instruments }));
import { InstrumentPicker } from "./instrument-picker.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function Host({ initial, disabled = false }: { initial: string[] | null; disabled?: boolean }) {
  const [value, setValue] = useState(initial);
  return <InstrumentPicker agentId="agent-fixture" value={value} disabled={disabled} onChange={(next) => { mocks.change(next); setValue(next); }} />;
}

describe("Instrument picker", () => {
  let root: Root, container: HTMLDivElement;
  const dialogMethods = Object.fromEntries(["showModal", "close"].map((key) => [key, Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, key)]));
  beforeAll(() => {
    Object.defineProperties(HTMLDialogElement.prototype, {
      showModal: { configurable: true, value(this: HTMLDialogElement) { this.open = true; } },
      close: { configurable: true, value(this: HTMLDialogElement) { this.open = false; } },
    });
  });
  afterAll(() => {
    for (const [key, descriptor] of Object.entries(dialogMethods)) {
      if (descriptor) Object.defineProperty(HTMLDialogElement.prototype, key, descriptor);
      else Reflect.deleteProperty(HTMLDialogElement.prototype, key);
    }
  });
  beforeEach(async () => {
    vi.resetAllMocks();
    mocks.instruments.mockResolvedValue({ ok: true, data: ["BTC", "ETH", "SOL"] });
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(<Host initial={["BTC"]} />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  const dialog = () => document.querySelector("dialog")!;
  const button = (text: string) => [...document.querySelectorAll("button")].find((entry) => entry.textContent === text)!;
  const click = async (text: string) => act(async () => button(text).click());
  const checkbox = (symbol: string) => dialog().querySelector<HTMLInputElement>(`input[type="checkbox"][aria-label="${symbol}"]`)!;
  const toggle = async (symbol: string) => act(async () => checkbox(symbol).click());
  const mode = async (unlimited: boolean) => act(async () => dialog().querySelectorAll<HTMLInputElement>('input[type="radio"]')[unlimited ? 0 : 1]!.click());
  async function search(value: string) {
    await act(async () => {
      const input = dialog().querySelector<HTMLInputElement>('input[type="search"]')!;
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  }
  it("shows selected badges with lazy logos and loads the venue catalog only when opened", async () => {
    expect(container.textContent).toContain("BTC");
    expect(container.querySelector('img[loading="lazy"]')).not.toBeNull();
    expect(mocks.instruments).not.toHaveBeenCalled();
    await click("Select instruments");
    expect(dialog().open).toBe(true);
    expect(mocks.instruments).toHaveBeenCalledWith({ data: { id: "agent-fixture" } });
    expect(checkbox("BTC").checked).toBe(true);
    expect(checkbox("ETH").checked).toBe(false);
    expect(dialog().textContent).toContain("1 / 50 selected");
  });
  it("filters case-insensitively without losing hidden selections, and applies exact symbols", async () => {
    await click("Select instruments");
    await search(" eTh ");
    expect(checkbox("BTC")).toBeNull();
    expect(checkbox("ETH")).not.toBeNull();
    await toggle("ETH");
    await search("not-a-symbol");
    expect(dialog().textContent).toContain("No instruments match this filter.");
    await search("");
    expect(checkbox("BTC").checked).toBe(true);
    expect(checkbox("ETH").checked).toBe(true);
    await click("Apply");
    expect(mocks.change).toHaveBeenCalledWith(["BTC", "ETH"]);
    expect(document.querySelector("dialog")).toBeNull();
    expect(container.textContent).toContain("ETH");
  });
  it.each(["Cancel", "Escape"])("discards changes on %s and restores the last applied selection when reopened", async (action) => {
    await click("Select instruments");
    await toggle("ETH");
    if (action === "Cancel") await click("Cancel");
    else await act(async () => dialog().dispatchEvent(new Event("cancel", { cancelable: true })));
    expect(mocks.change).not.toHaveBeenCalled();
    expect(document.querySelector("dialog")).toBeNull();
    await click("Select instruments");
    expect(checkbox("BTC").checked).toBe(true);
    expect(checkbox("ETH").checked).toBe(false);
  });
  it("supports bulk selection and refuses to interpret Select none as unrestricted trading", async () => {
    await click("Select instruments");
    await click("Select all");
    expect(["BTC", "ETH", "SOL"].every((symbol) => checkbox(symbol).checked)).toBe(true);
    await click("Select none");
    expect(button("Apply").disabled).toBe(true);
    expect(dialog().textContent).toContain("Select at least one instrument");
    await click("Apply");
    expect(mocks.change).not.toHaveBeenCalled();
    await mode(true);
    expect(button("Apply").disabled).toBe(false);
    expect(checkbox("BTC").matches(":disabled")).toBe(true);
    await click("Apply");
    expect(mocks.change).toHaveBeenCalledWith(null);
    expect(container.textContent).toContain("All instruments · No limit");
  });
  it("opens unrestricted settings in No limit mode and requires a selection when restricting", async () => {
    await act(async () => root.render(<Host key="unlimited" initial={null} />));
    await click("Select instruments");
    expect(dialog().querySelector<HTMLInputElement>('input[type="radio"]')!.checked).toBe(true);
    await mode(false);
    expect(button("Apply").disabled).toBe(true);
    await toggle("SOL");
    await click("Apply");
    expect(mocks.change).toHaveBeenCalledWith(["SOL"]);
  });
  it("enforces the existing 50-instrument cap and bulk-selects only filtered matches", async () => {
    const symbols = Array.from({ length: 51 }, (_, index) => `COIN${String(index).padStart(2, "0")}`);
    mocks.instruments.mockResolvedValue({ ok: true, data: symbols });
    await act(async () => root.render(<Host key="capped" initial={symbols.slice(0, 50)} />));
    await click("Select instruments");
    expect(button("Select all").disabled).toBe(true);
    expect(checkbox("COIN50").disabled).toBe(true);
    await toggle("COIN00");
    expect(checkbox("COIN50").disabled).toBe(false);
    await toggle("COIN50");
    expect(checkbox("COIN00").disabled).toBe(true);
    await click("Select none");
    await search("COIN0");
    await click("Select all shown");
    await click("Apply");
    expect(mocks.change).toHaveBeenCalledWith(symbols.slice(0, 10));
  });
  it("preserves saved unavailable symbols until explicitly removed", async () => {
    await act(async () => root.render(<Host key="delisted" initial={["BTC", "OLD"]} />));
    await click("Select instruments");
    expect(checkbox("OLD").checked).toBe(true);
    expect(checkbox("OLD").closest("label")?.textContent).toContain("Unavailable");
    await toggle("ETH");
    await click("Apply");
    expect(mocks.change).toHaveBeenLastCalledWith(["BTC", "ETH", "OLD"]);
    await click("Select instruments");
    await toggle("OLD");
    expect(checkbox("OLD").disabled).toBe(true);
    await click("Apply");
    expect(mocks.change).toHaveBeenLastCalledWith(["BTC", "ETH"]);
  });
  it("handles catalog failure and retry without changing saved restrictions", async () => {
    mocks.instruments.mockResolvedValueOnce({ ok: false, error: "Venue unavailable" });
    await click("Select instruments");
    expect(dialog().querySelector('[role="alert"]')!.textContent).toContain("Venue unavailable");
    expect(button("Apply").disabled).toBe(true);
    expect(mocks.change).not.toHaveBeenCalled();
    await click("Retry");
    expect(checkbox("BTC").checked).toBe(true);
    expect(button("Apply").disabled).toBe(false);
  });
  it("shows an empty catalog without turning saved restrictions into No limit", async () => {
    mocks.instruments.mockResolvedValue({ ok: true, data: [] });
    await click("Select instruments");
    expect(checkbox("BTC").checked).toBe(true);
    expect(checkbox("BTC").closest("label")?.textContent).toContain("Unavailable");
    await search("nothing");
    expect(dialog().textContent).toContain("No active perpetual instruments are available.");
    expect(mocks.change).not.toHaveBeenCalled();
  });
  it("does not open or fetch when disabled", async () => {
    await act(async () => root.render(<Host initial={["BTC"]} disabled />));
    expect(button("Select instruments").disabled).toBe(true);
    await click("Select instruments");
    expect(document.querySelector("dialog")).toBeNull();
    expect(mocks.instruments).not.toHaveBeenCalled();
  });
  it("ignores a catalog response after cancellation", async () => {
    let resolve!: (value: unknown) => void;
    mocks.instruments.mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
    await click("Select instruments");
    expect(dialog().textContent).toContain("Loading instruments");
    await click("Cancel");
    await act(async () => resolve({ ok: true, data: ["ETH"] }));
    expect(document.querySelector("dialog")).toBeNull();
    expect(mocks.change).not.toHaveBeenCalled();
  });
});

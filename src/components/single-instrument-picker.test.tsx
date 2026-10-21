// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ catalog: vi.fn(), change: vi.fn() }));
vi.mock("../web/charts.js", () => ({ getChartInstruments: mocks.catalog }));
import { SingleInstrumentPicker } from "./instrument-picker.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function Host({ initial = "BTC", instruments = ["BTC", "ETH", "SOL"], disabled = false }: { initial?: string; instruments?: string[]; disabled?: boolean }) {
  const [value, setValue] = useState(initial);
  return <SingleInstrumentPicker instruments={instruments} value={value} disabled={disabled} onChange={async (next) => {
    const saved = await mocks.change(next) as boolean;
    if (saved) setValue(next);
    return saved;
  }} />;
}

describe("single chart instrument picker", () => {
  let root: Root, container: HTMLDivElement;
  const methods = Object.fromEntries(["showModal", "close"].map((key) => [key, Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, key)]));
  beforeAll(() => {
    Object.defineProperties(HTMLDialogElement.prototype, {
      showModal: { configurable: true, value(this: HTMLDialogElement) { this.open = true; } },
      close: { configurable: true, value(this: HTMLDialogElement) { this.open = false; } },
    });
  });
  afterAll(() => { for (const [key, descriptor] of Object.entries(methods)) { if (descriptor) Object.defineProperty(HTMLDialogElement.prototype, key, descriptor); else Reflect.deleteProperty(HTMLDialogElement.prototype, key); } });
  beforeEach(async () => {
    vi.resetAllMocks(); mocks.change.mockResolvedValue(true);
    Element.prototype.scrollIntoView = vi.fn();
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(<Host />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  const dialog = () => document.querySelector("dialog")!;
  const trigger = () => container.querySelector<HTMLButtonElement>("[aria-label=Instrument]")!;
  const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find((entry) => entry.textContent === text)!;
  const choose = (symbol: string) => act(async () => dialog().querySelector<HTMLInputElement>(`input[type=radio][aria-label='${symbol}']`)!.click());
  const open = () => act(async () => trigger().click());
  const click = (text: string) => act(async () => button(text).click());
  async function search(value: string) {
    await act(async () => {
      const input = dialog().querySelector<HTMLInputElement>("input[type=search]")!;
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  }

  async function key(value: string, options: KeyboardEventInit = {}) {
    let prevented = false;
    await act(async () => {
      const input = dialog().querySelector<HTMLInputElement>("input[type=search]")!;
      input.focus();
      const event = new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true, ...options });
      input.dispatchEvent(event); prevented = event.defaultPrevented;
    });
    return prevented;
  }
  it("automatically highlights the first search result and applies it immediately with Enter", async () => {
    await open(); await search(" eth ");
    expect(dialog().querySelector<HTMLInputElement>("input[aria-label=ETH]")!.checked).toBe(true);
    expect(mocks.change).not.toHaveBeenCalled();
    expect(await key("Enter")).toBe(true);
    expect(mocks.change).toHaveBeenCalledExactlyOnceWith("ETH");
    expect(document.querySelector("dialog")).toBeNull(); expect(trigger().textContent).toContain("ETH");
  });
  it("moves through only filtered results with arrow keys, keeps search focus and stops at the boundaries", async () => {
    await act(async () => root.render(<Host key="keyboard" initial="SOL" />));
    await open(); await search("t");
    expect(dialog().querySelector<HTMLInputElement>("input[aria-label=BTC]")!.checked).toBe(true);
    const input = dialog().querySelector("input[type=search]")!;
    expect(await key("ArrowDown")).toBe(true);
    expect(dialog().querySelector<HTMLInputElement>("input[aria-label=ETH]")!.checked).toBe(true);
    expect(document.activeElement).toBe(input);
    expect(Element.prototype.scrollIntoView).toHaveBeenLastCalledWith({ block: "nearest" });
    await key("ArrowDown"); expect(dialog().querySelector<HTMLInputElement>("input[aria-label=ETH]")!.checked).toBe(true);
    await key("ArrowUp"); await key("ArrowUp");
    expect(dialog().querySelector<HTMLInputElement>("input[aria-label=BTC]")!.checked).toBe(true);
    await key("ArrowDown");
    expect(mocks.change).not.toHaveBeenCalled();
    await key("Enter"); expect(mocks.change).toHaveBeenCalledExactlyOnceWith("ETH");
  });
  it("resets the highlight on each search change and never submits a hidden selection when there are no matches", async () => {
    await open(); await search("t"); await key("ArrowDown");
    await search("so"); expect(dialog().querySelector<HTMLInputElement>("input[aria-label=SOL]")!.checked).toBe(true);
    await search("not-a-symbol");
    expect(button("Apply").disabled).toBe(true);
    await key("ArrowDown"); await key("ArrowUp"); await key("Enter");
    expect(mocks.change).not.toHaveBeenCalled(); expect(dialog().open).toBe(true);
    await search("e"); await key("Enter"); expect(mocks.change).toHaveBeenCalledExactlyOnceWith("ETH");
  });
  it("skips unavailable instruments during keyboard navigation", async () => {
    await act(async () => root.render(<Host key="delisted-keyboard" initial="OLD" />));
    await open(); await search("o");
    expect(dialog().querySelector<HTMLInputElement>("input[aria-label=OLD]")!.disabled).toBe(true);
    expect(dialog().querySelector<HTMLInputElement>("input[aria-label=SOL]")!.checked).toBe(true);
    await key("ArrowUp"); await key("ArrowDown"); await key("Enter");
    expect(mocks.change).toHaveBeenCalledExactlyOnceWith("SOL");
  });
  it("does not apply or navigate during IME composition, or apply on repeated Enter", async () => {
    await open(); await search("t");
    await key("ArrowDown", { isComposing: true });
    expect(dialog().querySelector<HTMLInputElement>("input[aria-label=BTC]")!.checked).toBe(true);
    await key("Enter", { isComposing: true }); await key("Enter", { repeat: true });
    expect(mocks.change).not.toHaveBeenCalled();
    await key("ArrowDown"); await key("Enter"); expect(mocks.change).toHaveBeenCalledExactlyOnceWith("ETH");
  });
  it("supports Enter when navigating the radio options with Tab", async () => {
    await open(); await choose("ETH");
    await act(async () => dialog().querySelector("input[aria-label=ETH]")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    expect(mocks.change).toHaveBeenCalledExactlyOnceWith("ETH");
  });
  it("shows the current logo and opens a searchable, icon-rich modal without fetching again", async () => {
    expect(trigger().querySelector("img")!.getAttribute("src")).toBe("/currency/BTC.svg");
    expect(trigger().getAttribute("aria-haspopup")).toBe("dialog");
    await open();
    expect(dialog().open).toBe(true); expect(dialog().textContent).toContain("Select chart instrument");
    expect(dialog().querySelector("input[type=search]")).not.toBeNull();
    expect(dialog().querySelectorAll("img[loading=lazy]")).toHaveLength(3);
    expect(dialog().querySelectorAll("input[type=checkbox]")).toHaveLength(0);
    expect(dialog().textContent).not.toContain("Select all"); expect(dialog().textContent).not.toContain("No limit");
    expect(mocks.catalog).not.toHaveBeenCalled();
  });
  it("selects exactly one instrument and updates the selected instrument's logo after Apply", async () => {
    await open(); await choose("ETH"); await choose("SOL");
    expect(dialog().querySelectorAll("input[type=radio]:checked")).toHaveLength(1);
    expect(dialog().querySelector<HTMLInputElement>("input[aria-label=SOL]")!.checked).toBe(true);
    expect(mocks.change).not.toHaveBeenCalled();
    await click("Apply");
    expect(mocks.change).toHaveBeenCalledExactlyOnceWith("SOL"); expect(document.querySelector("dialog")).toBeNull();
    expect(trigger().textContent).toContain("SOL"); expect(trigger().querySelector("img")!.getAttribute("src")).toBe("/currency/SOL.svg");
  });
  it("filters trimmed, case-insensitive searches and selects the first result when the filter is cleared", async () => {
    await open(); await search(" eTh ");
    expect(dialog().querySelectorAll("input[type=radio]")).toHaveLength(1);
    await choose("ETH"); await search("not-a-symbol");
    expect(dialog().textContent).toContain("No instruments match this filter.");
    await search("");
    expect(dialog().querySelectorAll("input[type=radio]:checked")).toHaveLength(1);
    expect(dialog().querySelector<HTMLInputElement>("input[aria-label=BTC]")!.checked).toBe(true);
  });
  it.each(["Cancel", "Escape"])("discards the draft selection on %s", async (method) => {
    await open(); await choose("ETH");
    if (method === "Cancel") await click("Cancel");
    else await act(async () => dialog().dispatchEvent(new Event("cancel", { cancelable: true })));
    expect(mocks.change).not.toHaveBeenCalled(); expect(trigger().textContent).toContain("BTC");
    await open(); expect(dialog().querySelector<HTMLInputElement>("input[aria-label=BTC]")!.checked).toBe(true);
  });
  it("preserves an unavailable current symbol but only permits choosing active instruments", async () => {
    await act(async () => root.render(<Host key="unavailable" initial="OLD" />));
    await open();
    const old = dialog().querySelector<HTMLInputElement>("input[aria-label=OLD]")!;
    expect(old.checked).toBe(true); expect(old.disabled).toBe(true);
    expect(old.closest("label")!.textContent).toContain("Unavailable"); expect(button("Apply").disabled).toBe(true);
    await choose("ETH"); await click("Apply"); expect(mocks.change).toHaveBeenCalledWith("ETH");
  });
  it("keeps the dialog and the old instrument on save failure, and supports retry", async () => {
    mocks.change.mockResolvedValueOnce(false);
    await open(); await choose("ETH"); await click("Apply");
    expect(dialog().querySelector("[role=alert]")!.textContent).toContain("Unable to change instrument");
    expect(trigger().textContent).toContain("BTC");
    await click("Apply"); expect(document.querySelector("dialog")).toBeNull(); expect(trigger().textContent).toContain("ETH");
  });
  it("reports rejected saves without committing the draft instrument", async () => {
    mocks.change.mockRejectedValueOnce(new Error("Offline"));
    await open(); await choose("ETH"); await click("Apply");
    expect(dialog().querySelector("[role=alert]")!.textContent).toBe("Offline"); expect(trigger().textContent).toContain("BTC");
  });
  it("blocks duplicate submissions while applying", async () => {
    let complete!: (saved: boolean) => void;
    mocks.change.mockReturnValue(new Promise<boolean>((resolve) => { complete = resolve; }));
    await open(); await choose("ETH");
    await act(async () => { for (let i = 0; i < 2; i++) dialog().querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
    expect(mocks.change).toHaveBeenCalledTimes(1); expect(button("Apply").disabled).toBe(true);
    expect(dialog().querySelector("input[type=search]")!.matches(":disabled")).toBe(true);
    await act(async () => complete(true)); expect(document.querySelector("dialog")).toBeNull();
  });
  it("does not open when disabled and handles an empty catalog without inventing a selection", async () => {
    await act(async () => root.render(<Host disabled />)); await open();
    expect(document.querySelector("dialog")).toBeNull();
    await act(async () => root.render(<Host instruments={[]} />)); await open();
    expect(dialog().textContent).toContain("No active perpetual instruments are available.");
    expect(button("Apply").disabled).toBe(true); expect(mocks.change).not.toHaveBeenCalled();
  });
});

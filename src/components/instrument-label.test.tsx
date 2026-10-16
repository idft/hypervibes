// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { InstrumentLabel } from "./instrument-label.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("instrument labels", () => {
  let root: Root;
  let container: HTMLDivElement;
  beforeEach(() => { container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  it("shows a decorative, lazy same-origin logo beside the unchanged symbol", async () => {
    await act(async () => root.render(<InstrumentLabel symbol="BTC" />));
    const image = container.querySelector("img")!;
    expect(image.getAttribute("src")).toBe("/currency/BTC.svg");
    expect(image.getAttribute("alt")).toBe("");
    expect(image.getAttribute("loading")).toBe("lazy");
    expect(image.getAttribute("width")).toBe("20");
    expect(container.textContent).toBe("BTC");
    await act(async () => image.dispatchEvent(new Event("error")));
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector('[aria-hidden="true"]')?.textContent).toBe("BTC");
    await act(async () => root.render(<InstrumentLabel symbol="ETH" />));
    expect(container.querySelector("img")?.getAttribute("src")).toBe("/currency/ETH.svg");
  });
  it.each(["@107", "BTC/USDC", "<script>"])("leaves unsupported identifiers as plain text: %s", async (symbol) => {
    await act(async () => root.render(<InstrumentLabel symbol={symbol} />));
    expect(container.querySelector("img")).toBeNull();
    expect(container.textContent).toBe(symbol);
  });
});

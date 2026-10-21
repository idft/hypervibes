// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { InstrumentLabel } from "./instrument-label.js";

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
  it("shows an icon with an accessible symbol and tooltip in icon-only mode", async () => {
    await act(async () => root.render(<InstrumentLabel symbol="BTC" iconOnly />));
    expect(container.querySelector("img")?.getAttribute("src")).toBe("/currency/BTC.svg");
    expect(container.querySelector('[title="BTC"]')).not.toBeNull();
    expect(container.querySelector(".sr-only")?.textContent).toBe("BTC");
    await act(async () => container.querySelector("img")!.dispatchEvent(new Event("error")));
    expect(container.querySelector('[aria-hidden="true"]')?.textContent).toBe("BTC");
    await act(async () => root.render(<InstrumentLabel symbol="@107" iconOnly />));
    expect(container.querySelector(".sr-only")).toBeNull();
    expect(container.textContent).toBe("@107");
  });
  it.each(["@107", "BTC/USDC", "<script>"])("leaves unsupported identifiers as plain text: %s", async (symbol) => {
    await act(async () => root.render(<InstrumentLabel symbol={symbol} />));
    expect(container.querySelector("img")).toBeNull();
    expect(container.textContent).toBe(symbol);
  });
});

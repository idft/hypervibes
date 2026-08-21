import { afterEach, describe, expect, it, vi } from "vitest";

import { initAgentPage } from "./page";

afterEach(() => {
  document.body.replaceChildren();
  vi.unstubAllGlobals();
});

describe("agent prompt editor", () => {
  it("shows the first prompt and switches forms when a prompt is selected", () => {
    document.body.innerHTML = `
      <section data-agent-prompts>
        <button type="button" data-agent-prompt-nav-item="analysis">Analysis</button>
        <button type="button" data-agent-prompt-nav-item="trading">Trading</button>
        <form data-agent-prompt-form="analysis" class="hidden"><textarea name="prompt"></textarea></form>
        <form data-agent-prompt-form="trading" class="hidden"><textarea name="prompt"></textarea></form>
      </section>
    `;

    initAgentPage();

    const analysis = document.querySelector<HTMLFormElement>('[data-agent-prompt-form="analysis"]');
    const trading = document.querySelector<HTMLFormElement>('[data-agent-prompt-form="trading"]');
    expect(analysis?.classList.contains("hidden")).toBe(false);
    expect(trading?.classList.contains("hidden")).toBe(true);

    document.querySelector<HTMLButtonElement>('[data-agent-prompt-nav-item="trading"]')?.click();
    expect(analysis?.classList.contains("hidden")).toBe(true);
    expect(trading?.classList.contains("hidden")).toBe(false);
  });

  it("keeps the selected prompt visible after importing a file", async () => {
    document.body.innerHTML = `
      <section data-agent-prompts>
        <button type="button" data-agent-prompt-nav-item="analysis">Analysis</button>
        <button type="button" data-agent-prompt-nav-item="trading">Trading</button>
        <form data-agent-prompt-form="analysis" class="hidden"><textarea name="prompt"></textarea></form>
        <form data-agent-prompt-form="trading" class="hidden">
          <textarea name="prompt"></textarea>
          <input type="file" data-agent-prompt-file-input="trading">
          <button type="button" data-agent-prompt-import="trading">Import</button>
          <button type="button" data-agent-prompt-reset="trading">Reset</button>
          <button type="submit" data-agent-prompt-save="trading">Save</button>
          <textarea id="default-trading-strategy-prompt-value"></textarea>
        </form>
      </section>
    `;

    initAgentPage();
    document.querySelector<HTMLButtonElement>('[data-agent-prompt-nav-item="trading"]')?.click();
    const fileInput = document.querySelector<HTMLInputElement>('[data-agent-prompt-file-input="trading"]');
    const file = new File(["updated prompt"], "prompt.txt", { type: "text/plain" });
    Object.defineProperty(fileInput, "files", { value: [file] });
    fileInput?.dispatchEvent(new Event("change"));
    await Promise.resolve();

    expect(document.querySelector('[data-agent-prompt-form="analysis"]')?.classList.contains("hidden")).toBe(true);
    expect(document.querySelector('[data-agent-prompt-form="trading"]')?.classList.contains("hidden")).toBe(false);
  });
});

describe("agent currency icons", () => {
  it("loads an icon when its currency row enters the scroll viewport", () => {
    let callback: IntersectionObserverCallback | undefined;
    class TestIntersectionObserver {
      constructor(next: IntersectionObserverCallback) {
        callback = next;
      }

      disconnect() {}
      observe() {}
      takeRecords() { return []; }
      unobserve() {}
    }
    vi.stubGlobal("IntersectionObserver", TestIntersectionObserver);
    document.body.innerHTML = `
      <section data-agent-instrument-selector>
        <button data-select-currencies></button>
        <div data-currency-modal class="hidden">
          <button data-currency-modal-close></button>
          <button data-apply-currencies></button>
          <button data-select-all-currencies></button>
          <button data-select-no-currencies></button>
          <div data-instrument-scroll-container>
          <label data-instrument-row data-instrument-label="BTC" data-instrument-logo-url="/currency/btc.svg">
            <input type="checkbox" name="instrument_id" value="BTC">
            <img data-instrument-logo alt="">
          </label>
        </div>
        </div>
        <div data-selected-instrument-list></div>
        <div data-selected-instrument-empty></div>
        <div data-no-currencies-warning></div>
      </section>
    `;

    initAgentPage();
    const row = document.querySelector<HTMLElement>("[data-instrument-row]");
    expect(row).not.toBeNull();
    if (!row) return;
    callback?.([{ isIntersecting: true, target: row } as unknown as IntersectionObserverEntry], {} as IntersectionObserver);

    expect(document.querySelector<HTMLImageElement>("[data-instrument-logo]")?.getAttribute("src")).toBe("/currency/btc.svg");
  });

  it("renders selected currency icons and saves modal selections", () => {
    document.body.innerHTML = `
      <section data-agent-instrument-selector>
        <button data-select-currencies>Select currencies</button>
        <div data-currency-modal class="hidden">
          <button data-currency-modal-close></button>
          <button type="submit" data-apply-currencies>Save</button>
          <button data-select-all-currencies>Select all</button>
          <button data-select-no-currencies>Select none</button>
          <input data-instrument-search>
          <div data-instrument-scroll-container>
            <label data-instrument-row data-instrument-label="BTC" data-instrument-logo-url="/currency/btc.svg"><input type="checkbox" name="instrument_id" value="BTC" checked></label>
            <label data-instrument-row data-instrument-label="ETH" data-instrument-logo-url="/currency/eth.svg"><input type="checkbox" name="instrument_id" value="ETH"></label>
          </div>
        </div>
        <div data-instrument-search-empty></div>
        <div data-selected-instrument-list></div>
        <div data-selected-instrument-empty></div>
        <div data-no-currencies-warning></div>
      </section>
    `;

    initAgentPage();
    expect(document.querySelector<HTMLImageElement>("[data-selected-instrument-list] img")?.getAttribute("src")).toBe("/currency/btc.svg");

    document.querySelector<HTMLButtonElement>("[data-select-currencies]")?.click();
    document.querySelector<HTMLButtonElement>("[data-select-all-currencies]")?.click();
    expect(document.querySelectorAll<HTMLInputElement>('input[name="instrument_id"]:checked')).toHaveLength(2);
    document.querySelector<HTMLButtonElement>("[data-select-no-currencies]")?.click();
    expect(document.querySelectorAll<HTMLInputElement>('input[name="instrument_id"]:checked')).toHaveLength(0);
    document.querySelector<HTMLInputElement>('input[value="ETH"]')?.click();
    expect(document.querySelector<HTMLButtonElement>("[data-apply-currencies]")?.type).toBe("submit");
    document.querySelector<HTMLButtonElement>("[data-apply-currencies]")?.click();

    expect(document.querySelectorAll("[data-selected-instrument-list] img")).toHaveLength(1);
    expect(document.querySelector<HTMLElement>("[data-currency-modal]")?.classList.contains("hidden")).toBe(true);
  });
});

describe("job detail modals", () => {
  it("opens, focuses, and closes the additional instructions modal", () => {
    document.body.innerHTML = `
      <button type="button" data-job-detail-modal-trigger="additional-instructions-modal">Additional Instructions</button>
      <div id="additional-instructions-modal" data-job-detail-modal class="hidden" aria-hidden="true">
        <button type="button" data-job-detail-modal-close>Close</button>
        <textarea data-job-detail-modal-initial-focus></textarea>
      </div>
    `;

    initAgentPage();
    const trigger = document.querySelector<HTMLButtonElement>("[data-job-detail-modal-trigger]");
    const modal = document.querySelector<HTMLElement>("[data-job-detail-modal]");
    const textarea = document.querySelector<HTMLTextAreaElement>("[data-job-detail-modal-initial-focus]");
    trigger?.click();

    expect(modal?.classList.contains("hidden")).toBe(false);
    expect(modal?.getAttribute("aria-hidden")).toBe("false");
    expect(document.activeElement).toBe(textarea);

    modal?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(modal?.classList.contains("hidden")).toBe(true);
    expect(document.activeElement).toBe(trigger);
  });
});

describe("new job form", () => {
  it("only requires a timeframe for candle-close jobs", () => {
    document.body.innerHTML = `
      <form data-new-job-form>
        <select data-new-job-kind>
          <option value="analysis" selected>Analysis</option>
          <option value="market_analysis">Market analysis</option>
        </select>
        <div data-new-job-timeframe><input data-new-job-timeframe-input required></div>
      </form>
    `;

    initAgentPage();
    const kind = document.querySelector<HTMLSelectElement>("[data-new-job-kind]");
    const timeframe = document.querySelector<HTMLElement>("[data-new-job-timeframe]");
    const input = document.querySelector<HTMLInputElement>("[data-new-job-timeframe-input]");

    kind!.value = "market_analysis";
    kind!.dispatchEvent(new Event("change"));

    expect(timeframe?.classList.contains("hidden")).toBe(true);
    expect(input?.required).toBe(false);
  });
});

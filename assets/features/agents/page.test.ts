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
        <div data-instrument-scroll-container>
          <label data-instrument-row data-instrument-label="BTC" data-instrument-logo-url="/currency/btc.svg">
            <input type="checkbox" name="instrument_id" value="BTC">
            <img data-instrument-logo alt="">
          </label>
        </div>
        <div data-selected-instrument-list></div>
        <div data-selected-instrument-empty></div>
        <div data-no-currencies-warning></div>
        <button data-save-currencies></button>
      </section>
    `;

    initAgentPage();
    const row = document.querySelector<HTMLElement>("[data-instrument-row]");
    expect(row).not.toBeNull();
    if (!row) return;
    callback?.([{ isIntersecting: true, target: row } as unknown as IntersectionObserverEntry], {} as IntersectionObserver);

    expect(document.querySelector<HTMLImageElement>("[data-instrument-logo]")?.getAttribute("src")).toBe("/currency/btc.svg");
  });
});

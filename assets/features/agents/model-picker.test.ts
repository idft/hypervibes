import { afterEach, describe, expect, it } from "vitest";

import { initModelPickers } from "./model-picker";

afterEach(() => {
  document.body.replaceChildren();
});

describe("model picker", () => {
  it("shows models from a provider matching the filter", () => {
    document.body.innerHTML = `
      <form>
        <div>
          <input name="model_selection" value="openai/gpt-4o">
          <div data-model-picker data-model-picker-mode="modal">
            <button type="button" data-model-picker-open></button>
            <span data-model-picker-label></span>
            <img data-model-picker-logo>
            <span data-model-picker-default-badge></span>
            <div class="hidden" data-model-picker-modal>
              <input data-model-picker-search>
              <button data-model-picker-provider data-provider-id="openai" data-search-text="OpenAI"></button>
              <button data-model-picker-provider data-provider-id="anthropic" data-search-text="Anthropic"></button>
              <button data-model-picker-option data-provider-id="openai" data-value="openai/gpt-4o" data-label="GPT-4o" data-search-text="OpenAI GPT-4o"></button>
              <button data-model-picker-option data-provider-id="anthropic" data-value="anthropic/claude-sonnet" data-label="Claude Sonnet" data-search-text="Anthropic Claude Sonnet"></button>
            </div>
          </div>
        </div>
      </form>
    `;

    initModelPickers();
    document.querySelector<HTMLElement>("[data-model-picker-open]")?.click();
    const search = document.querySelector<HTMLInputElement>("[data-model-picker-search]");
    if (!search) throw new Error("Model picker search input was not rendered");
    search.value = "claude";
    search.dispatchEvent(new Event("input"));

    expect(document.querySelector<HTMLElement>('[data-model-picker-option][data-provider-id="openai"]')?.classList.contains("hidden")).toBe(true);
    expect(document.querySelector<HTMLElement>('[data-model-picker-option][data-provider-id="anthropic"]')?.classList.contains("hidden")).toBe(false);
    expect(document.querySelector<HTMLElement>('[data-model-picker-provider][data-provider-id="anthropic"]')?.classList.contains("bg-zinc-900")).toBe(true);
  });
});

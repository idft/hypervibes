import { afterEach, describe, expect, it, vi } from "vitest";

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
              <button type="button" data-model-picker-provider data-provider-id="openai" data-search-text="OpenAI"></button>
              <button type="button" data-model-picker-provider data-provider-id="anthropic" data-search-text="Anthropic"></button>
              <button type="button" data-model-picker-option data-provider-id="openai" data-value="openai/gpt-4o" data-label="GPT-4o" data-search-text="OpenAI GPT-4o"></button>
              <button type="button" data-model-picker-option data-provider-id="anthropic" data-value="anthropic/claude-sonnet" data-label="Claude Sonnet" data-search-text="Anthropic Claude Sonnet"></button>
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

  it("shows advertised thinking modes and defaults to default", () => {
    document.body.innerHTML = `
      <form>
        <div>
          <input name="model_selection" value="openai/gpt-4o">
          <input name="model_variant" value="">
          <details data-model-picker>
            <span data-model-picker-label></span><img data-model-picker-logo><span data-model-picker-default-badge></span>
            <button type="button" data-model-picker-option data-value="openai/gpt-4o" data-label="GPT-4o"></button>
            <div data-model-picker-variant-area>
              <div data-model-picker-variant-panel data-model-value="openai/gpt-4o"><select data-model-picker-variant-select><option value="">default</option><option value="high">high</option><option value="low">low</option></select></div>
            </div>
            <p data-model-picker-variant-warning></p>
          </details>
        </div>
      </form>
    `;

    initModelPickers();

    const select = document.querySelector<HTMLSelectElement>("[data-model-picker-variant-select]");
    expect(select?.value).toBe("");
    expect(select?.textContent).toContain("default");
    expect(select?.textContent).toContain("high");
    expect(document.querySelector("[data-model-picker-label]")?.textContent).toBe("GPT-4o");
  });

  it("disables thinking mode for models without advertised variants", () => {
    document.body.innerHTML = `
      <form><div><input name="model_selection" value="openai/gpt-4o"><input name="model_variant" value=""><details data-model-picker><span data-model-picker-label></span><img data-model-picker-logo><span data-model-picker-default-badge></span><button type="button" data-model-picker-option data-value="openai/gpt-4o" data-label="GPT-4o"></button><div data-model-picker-variant-area><div data-model-picker-variant-unavailable><select disabled data-model-picker-variant-unavailable-select><option value="">default</option></select></div></div></details></div></form>
    `;

    initModelPickers();

    expect(document.querySelector<HTMLSelectElement>("[data-model-picker-variant-unavailable-select]")?.disabled).toBe(true);
  });

  it("resets thinking mode when selecting another model", () => {
    document.body.innerHTML = `
      <form><div><input name="model_selection" value="openai/gpt-4o"><input name="model_variant" value="high"><details data-model-picker><span data-model-picker-label></span><img data-model-picker-logo><span data-model-picker-default-badge></span><button type="button" data-model-picker-option data-value="openai/gpt-4o" data-label="GPT-4o"></button><button type="button" data-model-picker-option data-value="anthropic/claude" data-label="Claude"></button><div data-model-picker-variant-area><div data-model-picker-variant-panel data-model-value="openai/gpt-4o"><select data-model-picker-variant-select><option value="">default</option><option value="high">high</option></select></div><div data-model-picker-variant-panel data-model-value="anthropic/claude"><select data-model-picker-variant-select><option value="">default</option><option value="max">max</option></select></div></div></details></div></form>
    `;

    initModelPickers();
    document.querySelector<HTMLElement>('[data-model-picker-option][data-value="anthropic/claude"]')?.click();

    expect(document.querySelector<HTMLInputElement>('input[name="model_variant"]')?.value).toBe("");
    expect(document.querySelector<HTMLSelectElement>('[data-model-value="anthropic/claude"] select')?.value).toBe("");
  });

  it("commits both modal values on save and neither on cancel", () => {
    document.body.innerHTML = `
      <form><div><input name="model_selection" value="openai/gpt-4o"><input name="model_variant" value="high"><div data-model-picker data-model-picker-mode="modal" data-model-picker-submit-on-save="true"><button type="button" data-model-picker-open></button><span data-model-picker-label></span><img data-model-picker-logo><span data-model-picker-default-badge></span><div class="hidden" data-model-picker-modal><button type="button" data-model-picker-option data-provider-id="openai" data-value="openai/gpt-4o" data-label="GPT-4o"></button><button type="button" data-model-picker-option data-provider-id="anthropic" data-value="anthropic/claude" data-label="Claude"></button><div data-model-picker-variant-area><div data-model-picker-variant-panel data-model-value="openai/gpt-4o"><select data-model-picker-variant-select><option value="">default</option><option value="high">high</option></select></div><div data-model-picker-variant-panel data-model-value="anthropic/claude"><select data-model-picker-variant-select><option value="">default</option><option value="max">max</option></select></div></div><button type="button" data-model-picker-cancel></button><button type="button" data-model-picker-save></button></div></div></div></form>
    `;

    initModelPickers();
    const form = document.querySelector<HTMLFormElement>("form");
    if (!form) throw new Error("Model picker form was not rendered");
    const submit = vi.spyOn(form, "requestSubmit").mockImplementation(() => {});
    document.querySelector<HTMLElement>("[data-model-picker-open]")?.click();
    document.querySelector<HTMLElement>('[data-model-picker-option][data-value="anthropic/claude"]')?.click();
    document.querySelector<HTMLElement>("[data-model-picker-cancel]")?.click();
    expect(document.querySelector<HTMLInputElement>('input[name="model_selection"]')?.value).toBe("openai/gpt-4o");
    expect(document.querySelector<HTMLInputElement>('input[name="model_variant"]')?.value).toBe("high");
    document.querySelector<HTMLElement>("[data-model-picker-open]")?.click();
    document.querySelector<HTMLElement>('[data-model-picker-option][data-value="anthropic/claude"]')?.click();
    const select = document.querySelector<HTMLSelectElement>('[data-model-value="anthropic/claude"] select');
    if (!select) throw new Error("Thinking mode select was not rendered");
    select.value = "max";
    select.dispatchEvent(new Event("change"));
    document.querySelector<HTMLElement>("[data-model-picker-save]")?.click();

    expect(document.querySelector<HTMLInputElement>('input[name="model_selection"]')?.value).toBe("anthropic/claude");
    expect(document.querySelector<HTMLInputElement>('input[name="model_variant"]')?.value).toBe("max");
    expect(submit).toHaveBeenCalledOnce();
  });

  it("can commit a modal selection without submitting its form", () => {
    document.body.innerHTML = `
      <form><div><input name="model_selection" value=""><input name="model_variant" value=""><div data-model-picker data-model-picker-mode="modal"><button type="button" data-model-picker-open></button><span data-model-picker-label></span><img data-model-picker-logo><span data-model-picker-default-badge></span><div class="hidden" data-model-picker-modal><button type="button" data-model-picker-option data-provider-id="openai" data-value="openai/gpt-4o" data-label="GPT-4o"></button><button type="button" data-model-picker-save></button></div></div></div></form>
    `;

    initModelPickers();
    const form = document.querySelector<HTMLFormElement>("form");
    if (!form) throw new Error("Model picker form was not rendered");
    const submit = vi.spyOn(form, "requestSubmit").mockImplementation(() => {});
    document.querySelector<HTMLElement>("[data-model-picker-open]")?.click();
    document.querySelector<HTMLElement>("[data-model-picker-option]")?.click();
    document.querySelector<HTMLElement>("[data-model-picker-save]")?.click();

    expect(document.querySelector<HTMLInputElement>('input[name="model_selection"]')?.value).toBe("openai/gpt-4o");
    expect(submit).not.toHaveBeenCalled();
  });

  it("warns about an unavailable persisted thinking mode and blocks modal save", () => {
    document.body.innerHTML = `
      <form><div><input name="model_selection" value="openai/gpt-4o"><input name="model_variant" value="retired"><div data-model-picker data-model-picker-mode="modal"><button type="button" data-model-picker-open></button><span data-model-picker-label></span><img data-model-picker-logo><span data-model-picker-default-badge></span><div class="hidden" data-model-picker-modal><button type="button" data-model-picker-option data-provider-id="openai" data-value="openai/gpt-4o" data-label="GPT-4o"></button><div data-model-picker-variant-area><div data-model-picker-variant-panel data-model-value="openai/gpt-4o"><select data-model-picker-variant-select><option value="">default</option><option value="high">high</option></select></div></div><p class="hidden" data-model-picker-variant-warning></p><button type="button" data-model-picker-save></button></div></div></div></form>
    `;

    initModelPickers();
    const form = document.querySelector<HTMLFormElement>("form");
    if (!form) throw new Error("Model picker form was not rendered");
    const submit = vi.spyOn(form, "requestSubmit").mockImplementation(() => {});
    document.querySelector<HTMLElement>("[data-model-picker-open]")?.click();
    document.querySelector<HTMLElement>("[data-model-picker-save]")?.click();

    expect(document.querySelector("[data-model-picker-variant-warning]")?.textContent).toContain("no longer available");
    expect(submit).not.toHaveBeenCalled();
    expect(document.querySelector<HTMLInputElement>('input[name="model_variant"]')?.value).toBe("retired");
  });
});

import { afterEach, describe, expect, it, vi } from "vitest";

import { installAgentLiveLifecycle } from "./live";
import { initModelPickers } from "./model-picker";

afterEach(() => {
  document.body.replaceChildren();
});

describe("conversation composer shortcut", () => {
  it("does not submit an empty message when Enter is pressed", () => {
    document.body.innerHTML = `
      <form action="/agents/test-agent/chat/00000000-0000-0000-0000-000000000000/messages">
        <textarea></textarea>
      </form>
    `;
    installAgentLiveLifecycle();

    const textarea = document.querySelector<HTMLTextAreaElement>("textarea");
    if (!textarea) throw new Error("Conversation message input was not rendered");
    const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
    textarea.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
  });
});

describe("model-dependent buttons", () => {
  it("enables a disabled job after its model selection changes", () => {
    document.body.innerHTML = `
      <form action="/agents/test-agent/jobs/1/model">
        <input name="model_selection" value="">
      </form>
      <button class="border-zinc-800 text-zinc-500 cursor-not-allowed opacity-50" data-model-dependent-enable data-enabled="false" disabled></button>
    `;
    installAgentLiveLifecycle();

    const input = document.querySelector<HTMLInputElement>('input[name="model_selection"]');
    const button = document.querySelector<HTMLButtonElement>("[data-model-dependent-enable]");
    if (!input || !button) throw new Error("Model selection controls were not rendered");
    input.value = "openai/gpt-4o";
    input.dispatchEvent(new Event("change", { bubbles: true }));

    expect(button.disabled).toBe(false);
    expect(button.classList.contains("cursor-pointer")).toBe(true);
    expect(button.classList.contains("text-emerald-300")).toBe(true);
    expect(button.classList.contains("text-zinc-500")).toBe(false);
  });

  it("enables a disabled job when a model picker modal is saved", () => {
    document.body.innerHTML = `
      <form action="/agents/test-agent/jobs/1/model">
        <input name="model_selection" value="">
        <div data-model-picker data-model-picker-mode="modal">
          <button type="button" data-model-picker-open></button>
          <span data-model-picker-label></span>
          <img data-model-picker-logo>
          <span data-model-picker-default-badge></span>
          <div class="hidden" data-model-picker-modal>
            <button type="button" data-model-picker-option data-provider-id="openai" data-value="openai/gpt-4o" data-label="GPT-4o"></button>
            <button type="button" data-model-picker-save></button>
          </div>
        </div>
      </form>
      <button data-model-dependent-enable data-enabled="false" disabled></button>
    `;
    installAgentLiveLifecycle();
    initModelPickers();

    const form = document.querySelector<HTMLFormElement>("form");
    const open = document.querySelector<HTMLElement>("[data-model-picker-open]");
    const option = document.querySelector<HTMLElement>("[data-model-picker-option]");
    const save = document.querySelector<HTMLElement>("[data-model-picker-save]");
    const button = document.querySelector<HTMLButtonElement>("[data-model-dependent-enable]");
    if (!form || !open || !option || !save || !button) throw new Error("Model picker controls were not rendered");
    vi.spyOn(form, "requestSubmit").mockImplementation(() => {});

    open.click();
    option.click();
    save.click();

    expect(button.disabled).toBe(false);
  });
});

describe("position close confirmation", () => {
  it("opens a modal configured for the selected close action", () => {
    document.body.innerHTML = `
      <button type="button" data-position-close-trigger data-position-close-action="/agents/test/positions/BTC/close" data-position-close-message="Close BTC at market?" data-position-close-confirm="Close BTC">Close</button>
      <div id="position-close-modal" class="hidden">
        <p id="position-close-modal-body"></p>
        <button type="button" data-position-close-cancel>Cancel</button>
        <form id="position-close-form"><button id="position-close-confirm" type="submit">Close</button></form>
      </div>
    `;
    installAgentLiveLifecycle();

    const trigger = document.querySelector<HTMLElement>("[data-position-close-trigger]");
    const modal = document.getElementById("position-close-modal");
    const form = document.getElementById("position-close-form") as HTMLFormElement | null;
    const body = document.getElementById("position-close-modal-body");
    const confirm = document.getElementById("position-close-confirm");
    if (!trigger || !modal || !form || !body || !confirm) throw new Error("Position close modal was not rendered");

    trigger.click();

    expect(modal.classList.contains("flex")).toBe(true);
    expect(form.action).toBe("http://localhost:3000/agents/test/positions/BTC/close");
    expect(body.textContent).toBe("Close BTC at market?");
    expect(confirm.textContent).toBe("Close BTC");

    document.querySelector<HTMLElement>("[data-position-close-cancel]")?.click();
    expect(modal.classList.contains("hidden")).toBe(true);
  });
});

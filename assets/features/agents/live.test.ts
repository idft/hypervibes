import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("htmx.org", () => ({ default: { swap: vi.fn() } }));

import { installAgentLiveLifecycle } from "./live";
import { initModelPickers } from "./model-picker";

afterEach(() => {
  window.dispatchEvent(new Event("pageshow"));
  document.body.replaceChildren();
});

describe("conversation creation submissions", () => {
  const markup = '<form data-conversation-create action="/agents/test-agent/chat/conversations" method="post"><input name="model_selection" value="openai/test"><input name="model_variant" value="high"><input name="csrf_token" value="token"><textarea name="strategy_prompt">Draft</textarea><button type="submit" class="cursor-pointer">New conversation</button></form>';
  const submit = (form: HTMLFormElement) => {
    const event = new Event("submit", { bubbles: true, cancelable: true });
    form.dispatchEvent(event);
    return event;
  };
  const form = () => {
    const value = document.querySelector<HTMLFormElement>("form");
    if (!value) throw new Error("Missing creation form");
    return value;
  };

  it("allows the first POST and blocks repeated submissions without losing fields", () => {
    document.body.innerHTML = markup;
    installAgentLiveLifecycle();
    expect(submit(form()).defaultPrevented).toBe(false);
    expect(submit(form()).defaultPrevented).toBe(true);
    const button = form().querySelector("button");
    expect(button?.disabled).toBe(true);
    expect(button?.textContent).toBe("Creating…");
    expect(button?.classList.contains("cursor-wait")).toBe(true);
    expect(Object.fromEntries(new FormData(form()))).toEqual({ model_selection: "openai/test", model_variant: "high", csrf_token: "token", strategy_prompt: "Draft" });
  });

  it("keeps the guard and disabled button after SSE replaces the form", () => {
    document.body.innerHTML = markup;
    installAgentLiveLifecycle();
    submit(form());
    document.body.innerHTML = markup;
    document.dispatchEvent(new CustomEvent("htmx:sseMessage", { detail: {} }));
    expect(form().querySelector("button")?.disabled).toBe(true);
    expect(submit(form()).defaultPrevented).toBe(true);
  });

  it("admits one request when requestSubmit is repeated, and respects native validation", () => {
    document.body.innerHTML = markup;
    installAgentLiveLifecycle();
    let requests = 0;
    form().addEventListener("submit", (event) => {
      if (!event.defaultPrevented) requests += 1;
      event.preventDefault(); // Emulate navigation without jsdom's unimplemented POST.
    });
    const required = document.createElement("input");
    required.required = true;
    form().append(required);
    form().requestSubmit();
    expect(requests).toBe(0);
    required.value = "valid";
    form().requestSubmit();
    form().requestSubmit();
    expect(requests).toBe(1);
  });

  it.each(["htmx:afterRequest", "htmx:sendError", "htmx:timeout", "htmx:validation:halted"])("allows another creation after %s, including a replaced form", (name) => {
    document.body.innerHTML = markup;
    installAgentLiveLifecycle();
    const original = form();
    submit(original);
    const request = new CustomEvent("htmx:beforeRequest", { bubbles: true, cancelable: true, detail: { elt: original } });
    original.dispatchEvent(request);
    expect(request.defaultPrevented).toBe(false);
    const repeated = new CustomEvent("htmx:beforeRequest", { bubbles: true, cancelable: true, detail: { elt: original } });
    original.dispatchEvent(repeated);
    expect(repeated.defaultPrevented).toBe(true);
    document.body.innerHTML = markup;
    document.dispatchEvent(new CustomEvent("htmx:afterSwap", { detail: { target: document.body } }));
    document.dispatchEvent(new CustomEvent(name, { detail: { elt: original, successful: name === "htmx:afterRequest" } }));
    expect(form().querySelector("button")?.disabled).toBe(false);
    expect(form().querySelector("button")?.textContent).toBe("New conversation");
    expect(submit(form()).defaultPrevented).toBe(false);
  });

  it("does not lock an invalid form and resets on history restoration", () => {
    document.body.innerHTML = markup;
    installAgentLiveLifecycle();
    const input = document.createElement("input");
    input.required = true;
    form().append(input);
    submit(form());
    expect(form().querySelector("button")?.disabled).toBe(false);
    input.value = "valid";
    expect(submit(form()).defaultPrevented).toBe(false);
    window.dispatchEvent(new PageTransitionEvent("pageshow", { persisted: true }));
    expect(form().querySelector("button")?.disabled).toBe(false);
    expect(submit(form()).defaultPrevented).toBe(false);
  });
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

describe("conversation composer SSE updates", () => {
  const incomingComposer = '<form><textarea name="message" disabled></textarea><button type="submit" disabled class="cursor-not-allowed">Send</button></form>';

  it("keeps an unsent draft and updates the busy state", () => {
    document.body.innerHTML = '<div id="conversation-composer"><form><textarea name="message"></textarea><button type="submit" class="cursor-pointer">Send</button></form></div>';
    installAgentLiveLifecycle();

    const composer = document.getElementById("conversation-composer");
    const textarea = composer?.querySelector<HTMLTextAreaElement>('textarea[name="message"]');
    const button = composer?.querySelector<HTMLButtonElement>('button[type="submit"]');
    if (!composer || !textarea || !button) throw new Error("Conversation composer was not rendered");
    textarea.value = "Draft in progress";

    const busyEvent = new CustomEvent("htmx:sseBeforeMessage", { bubbles: true, cancelable: true, detail: { data: incomingComposer } });
    composer.dispatchEvent(busyEvent);

    expect(busyEvent.defaultPrevented).toBe(true);
    expect(composer.querySelector('textarea[name="message"]')).toBe(textarea);
    expect(textarea.value).toBe("Draft in progress");
    expect(textarea.disabled).toBe(true);
    expect(button.disabled).toBe(true);
    expect(button.className).toBe("cursor-not-allowed");

    const idleEvent = new CustomEvent("htmx:sseBeforeMessage", { bubbles: true, cancelable: true, detail: { data: '<form><textarea name="message"></textarea><button type="submit" class="cursor-pointer">Send</button></form>' } });
    composer.dispatchEvent(idleEvent);
    expect(idleEvent.defaultPrevented).toBe(true);
    expect(textarea.value).toBe("Draft in progress");
    expect(textarea.disabled).toBe(false);
    expect(button.disabled).toBe(false);
    expect(button.className).toBe("cursor-pointer");
  });

  it("keeps focus when an empty composer receives an SSE update", () => {
    document.body.innerHTML = '<div id="conversation-composer"><form><textarea name="message"></textarea><button type="submit">Send</button></form></div>';
    installAgentLiveLifecycle();

    const composer = document.getElementById("conversation-composer");
    const textarea = composer?.querySelector<HTMLTextAreaElement>('textarea[name="message"]');
    if (!composer || !textarea) throw new Error("Conversation composer was not rendered");
    textarea.focus();

    const event = new CustomEvent("htmx:sseBeforeMessage", { bubbles: true, cancelable: true, detail: { data: incomingComposer } });
    composer.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(composer.querySelector('textarea[name="message"]')).toBe(textarea);
    expect(textarea.disabled).toBe(true);
  });

  it("allows an unfocused empty composer to refresh normally", () => {
    document.body.innerHTML = '<div id="conversation-composer"><form><textarea name="message"></textarea><button type="submit">Send</button></form></div>';
    installAgentLiveLifecycle();

    const composer = document.getElementById("conversation-composer");
    if (!composer) throw new Error("Conversation composer was not rendered");
    const event = new CustomEvent("htmx:sseBeforeMessage", { bubbles: true, cancelable: true, detail: { data: incomingComposer } });
    composer.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(false);
  });
});

describe("model-dependent buttons", () => {
  it("enables a disabled sub-agent after its model selection changes", () => {
    document.body.innerHTML = `
      <form action="/agents/test-agent/sub-agents/1/model">
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

  it("enables a disabled sub-agent when a model picker modal is saved", () => {
    document.body.innerHTML = `
      <form action="/agents/test-agent/sub-agents/1/model">
        <input name="model_selection" value="">
        <div data-model-picker>
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

describe("run cancellation confirmation", () => {
  it("opens and closes the run cancellation modal", () => {
    document.body.innerHTML = `
      <button type="button" data-run-cancel-trigger>Cancel run</button>
      <div id="run-cancel-modal" class="hidden">
        <button type="button" data-run-cancel-close>Keep running</button>
        <form action="/agents/test/runs/1/cancel" method="post"><button type="submit">Cancel run</button></form>
      </div>
    `;
    installAgentLiveLifecycle();

    const trigger = document.querySelector<HTMLElement>("[data-run-cancel-trigger]");
    const modal = document.getElementById("run-cancel-modal");
    if (!trigger || !modal) throw new Error("Run cancellation controls were not rendered");

    trigger.click();
    expect(modal.classList.contains("flex")).toBe(true);

    document.querySelector<HTMLElement>("[data-run-cancel-close]")?.click();
    expect(modal.classList.contains("hidden")).toBe(true);
  });
});

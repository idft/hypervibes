import { afterEach, describe, expect, it } from "vitest";

import { initSingletonRoleForms } from "./singleton-role-form";

afterEach(() => {
  document.body.replaceChildren();
});

describe("singleton role form", () => {
  it("requires a model and prompt before enabling Create", () => {
    document.body.innerHTML = `
      <form data-singleton-role-form>
        <input name="model_selection" value="">
        <textarea name="prompt"></textarea>
        <button type="submit" data-singleton-role-submit disabled></button>
      </form>
    `;

    initSingletonRoleForms();
    const model = document.querySelector<HTMLInputElement>('input[name="model_selection"]');
    const prompt = document.querySelector<HTMLTextAreaElement>('textarea[name="prompt"]');
    const submit = document.querySelector<HTMLButtonElement>("[data-singleton-role-submit]");
    if (!model || !prompt || !submit) throw new Error("Singleton role form was not rendered");

    expect(submit.disabled).toBe(true);

    model.value = "openai/gpt-4o";
    model.dispatchEvent(new Event("change", { bubbles: true }));
    expect(submit.disabled).toBe(true);

    prompt.value = "Trade with defined risk";
    prompt.dispatchEvent(new Event("input", { bubbles: true }));
    expect(submit.disabled).toBe(false);

    model.value = "";
    model.dispatchEvent(new Event("change", { bubbles: true }));
    expect(submit.disabled).toBe(true);
  });
});

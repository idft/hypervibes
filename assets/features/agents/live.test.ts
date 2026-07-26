import { describe, expect, it } from "vitest";

import { installAgentLiveLifecycle } from "./live";

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

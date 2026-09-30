import { afterEach, describe, expect, it, vi } from "vitest";
import htmx from "htmx.org";
import { installChatPermissions } from "./chat-permissions";

vi.mock("htmx.org", () => ({ default: { swap: vi.fn() } }));

installChatPermissions();

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
  vi.clearAllMocks();
});

function setup() {
  document.body.innerHTML = '<div id="conversation-summary"><button data-chat-permissions-open>Manage permissions</button><dialog id="chat-permissions-modal"><form data-chat-permissions-form><fieldset><select name="indicator_writes_policy"><option value="confirm" selected>Confirm</option><option value="allow">Allow</option></select></fieldset><button type="button" data-chat-permissions-cancel>Cancel</button><button type="submit">Save</button></form></dialog></div>';
  const summary = document.getElementById("conversation-summary");
  const dialog = document.querySelector("dialog");
  const select = document.querySelector("select");
  const trigger = document.querySelector<HTMLButtonElement>("[data-chat-permissions-open]");
  if (!summary || !dialog || !select || !trigger) throw new Error("Missing permissions controls");
  // Simulate native dialog lifecycle; jsdom does not implement modal display.
  dialog.showModal = vi.fn(() => { dialog.open = true; });
  dialog.close = vi.fn(() => { dialog.open = false; dialog.dispatchEvent(new Event("close")); });
  return { summary, dialog, select, trigger };
}

describe("chat permissions modal", () => {
  it("discards changes when cancelled and reopens with saved permissions", () => {
    const { dialog, select, trigger } = setup();
    trigger.click();
    expect(dialog.open).toBe(true);
    select.value = "allow";
    document.querySelector<HTMLButtonElement>("[data-chat-permissions-cancel]")?.click();
    expect(dialog.open).toBe(false);
    expect(select.value).toBe("confirm");
    trigger.click();
    expect(select.value).toBe("confirm");
  });

  it("preserves edits across SSE, honors busy state, and applies the latest snapshot on dismissal", () => {
    const { summary, dialog, select, trigger } = setup();
    trigger.click();
    select.value = "allow";
    const busy = '<button data-chat-permissions-open disabled>Manage permissions</button>';
    const idle = '<button data-chat-permissions-open>Manage permissions</button>';
    for (const [data, disabled] of [[busy, true], [idle, false]] as const) {
      const event = new CustomEvent("htmx:sseBeforeMessage", { bubbles: true, cancelable: true, detail: { data } });
      summary.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(true);
      expect(dialog.open).toBe(true);
      expect(select.value).toBe("allow");
      expect(dialog.querySelector("fieldset")?.disabled).toBe(disabled);
      expect(dialog.querySelector<HTMLButtonElement>('button[type="submit"]')?.disabled).toBe(disabled);
    }
    dialog.close(); // Native Escape dismissal also fires close.
    expect(select.value).toBe("confirm");
    expect(htmx.swap).toHaveBeenCalledOnce();
    expect(htmx.swap).toHaveBeenCalledWith(summary, idle, expect.objectContaining({ swapStyle: "innerHTML" }));
  });

  it("allows normal live refreshes while the modal is closed", () => {
    const { summary } = setup();
    const event = new CustomEvent("htmx:sseBeforeMessage", { bubbles: true, cancelable: true, detail: { data: "Updated summary" } });
    summary.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  });
});

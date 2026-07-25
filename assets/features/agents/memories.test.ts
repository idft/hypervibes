import { afterEach, describe, expect, it } from "vitest";

import { restoreSelectedMemoryItems } from "./memories";

afterEach(() => {
  document.body.replaceChildren();
});

describe("memory timeline selection", () => {
  it("marks the detail memory as selected in the timeline", () => {
    document.body.innerHTML = `
      <section data-agent-memories data-selected-memory-id="stale">
        <button data-memory-timeline-item data-memory-id="first" aria-pressed="true"></button>
        <button data-memory-timeline-item data-memory-id="second" aria-pressed="false"></button>
        <div id="memory-detail" data-memory-detail-id="second"></div>
      </section>
    `;

    restoreSelectedMemoryItems();

    const items = document.querySelectorAll<HTMLElement>("[data-memory-timeline-item]");
    expect(items[0].getAttribute("aria-pressed")).toBe("false");
    expect(items[1].getAttribute("aria-pressed")).toBe("true");
  });
});

import { afterEach, describe, expect, it } from "vitest";

import "./htmx";

afterEach(() => {
  document.body.replaceChildren();
});

describe("SSE cleanup", () => {
  it("removes a detached stream's reconnect URL", () => {
    const stream = document.createElement("div");
    stream.setAttribute("sse-connect", "/agents/test/live/stream");

    document.dispatchEvent(new CustomEvent("htmx:beforeCleanupElement", { detail: { elt: stream } }));

    expect(stream.hasAttribute("sse-connect")).toBe(false);
  });
});

import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("htmx.org", () => ({ default: {} }));
vi.mock("htmx-ext-sse", () => ({}));

import "./htmx";

afterEach(() => {
  document.body.replaceChildren();
});

describe("SSE cleanup", () => {
  it("closes a removed stream and disables reconnects", () => {
    const stream = document.createElement("div");
    stream.setAttribute("sse-connect", "/agents/test/live/stream");
    document.body.append(stream);
    const close = vi.fn();

    stream.dispatchEvent(new CustomEvent("htmx:sseOpen", { bubbles: true, detail: { source: { close } } }));

    document.dispatchEvent(new CustomEvent("htmx:beforeCleanupElement", { detail: { elt: stream } }));

    expect(close).toHaveBeenCalledOnce();
    expect(stream.hasAttribute("sse-connect")).toBe(false);
  });

  it("closes an orphaned stream before the next request", () => {
    const stream = document.createElement("div");
    stream.setAttribute("sse-connect", "/agents/test/live/stream");
    document.body.append(stream);
    const close = vi.fn();
    stream.dispatchEvent(new CustomEvent("htmx:sseOpen", { bubbles: true, detail: { source: { close } } }));
    stream.remove();

    document.dispatchEvent(new CustomEvent("htmx:beforeRequest"));

    expect(close).toHaveBeenCalledOnce();
    expect(stream.hasAttribute("sse-connect")).toBe(false);
  });
});

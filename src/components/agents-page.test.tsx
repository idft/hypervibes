import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { AgentsPage } from "./agents-page.js";

describe("Agent management", () => {
  it("offers creation and links to the full create page", () => {
    const html = renderToStaticMarkup(<AgentsPage agents={[]} />);
    expect(html).toContain("Agents</h1>");
    expect(html).toContain("No agents yet");
    expect(html).toContain("Create a new agent");
    expect(html).toContain("agents/new");
    expect(html).toContain("cursor-pointer");
    expect(html).not.toContain("hx-");
  });
  it("links records by immutable ID, escapes untrusted names and shows trading linkage", () => {
    const html = renderToStaticMarkup(<AgentsPage agents={[
      {
        id: "fixture-id", userId: "owner", name: "<script>bad()</script>", state: "failed", conversationId: null,
        error: "fixture", initialProvider: null, initialModelId: null, tradingAccountAddress: null, creationRequestId: "request", createdAt: 0, updatedAt: 0,
      },
      {
        id: "trading-id", userId: "owner", name: "Trading agent", state: "ready", conversationId: 2,
        error: null, initialProvider: null, initialModelId: null, tradingAccountAddress: "0x1234000000000000000000000000000000000000", creationRequestId: "request2", createdAt: 0, updatedAt: 0,
      },
    ]} />);
    expect(html).toContain('/agents/fixture-id');
    expect(html).toContain("Initialization failed");
    expect(html).toContain("Chat only");
    expect(html).toContain("0x1234000000000000000000000000000000000000");
    expect(html).not.toContain("<script>");
  });
});
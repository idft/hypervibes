import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { AgentsPage } from "./agents-page.js";

const create = async () => ({ ok: false, error: "fixture" });
describe("Agent management", () => {
  it("offers creation while clearly excluding trading", () => {
    const html = renderToStaticMarkup(<AgentsPage agents={[]} create={create} />);
    expect(html).toContain("Agents</h1>");
    expect(html).toContain("No agents yet");
    expect(html).toContain("Create a new agent");
    expect(html).toContain("Trading is not configured");
    expect(html).toContain("Agent name");
    expect(html).toContain("cursor-pointer");
    expect(html).not.toContain("hx-");
  });
  it("links records by immutable ID and escapes untrusted names", () => {
    const html = renderToStaticMarkup(<AgentsPage agents={[{
      id: "fixture-id", userId: "owner", name: "<script>bad()</script>", state: "failed", conversationId: null,
      error: "fixture", creationRequestId: "request", createdAt: 0, updatedAt: 0,
    }]} create={create} />);
    expect(html).toContain('/agents/fixture-id');
    expect(html).toContain("Initialization failed");
    expect(html).not.toContain("<script>");
  });
});

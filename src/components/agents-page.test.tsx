import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { AgentsPage } from "./agents-page.js";

describe("agents preview", () => {
  it("renders the existing visual structure with honest unavailable actions", () => {
    const html = renderToStaticMarkup(<AgentsPage status={{ database: "connected", agentExecution: "not-configured" }} />);
    expect(html).toContain("Agents</h1>");
    expect(html).toContain("No agents yet");
    expect(html).toContain("SQLite connected");
    expect(html).toContain("no trading is enabled");
    expect(html).toContain('disabled=""');
    expect(html).not.toContain("hx-");
  });
});

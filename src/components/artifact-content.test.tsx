import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { ArtifactContent, artifactContentUrl } from "./artifact-content.js";
import { ToolActivity, ToolCall } from "./tool-activity.js";
const id = "00000000-0000-0000-0000-000000000001";
describe("safe artifact rendering", () => {
  it("escapes active content and embeds only resolved image attachments", () => {
    const html = renderToStaticMarkup(<ArtifactContent agentId="root" content={`<script>alert(1)</script>\n![Chart](artifact:${id})\n![External](https://example.test/tracker.png)`} attachments={[{ id, title: "Chart", mediaType: "image/png" }]} />);
    expect(html).not.toContain("<script>"); expect(html).toContain("&lt;script&gt;");
    expect(html).toContain(`/api/agents/root/artifacts/${id}/content?inline=1`);
    expect(html).not.toContain('src="https://example.test');
    expect(html).toContain('alt="Chart"');
  });
  it("renders unresolved references as downloads, never removed page links or external images", () => {
    const html = renderToStaticMarkup(<ArtifactContent agentId="root" content={`![Missing](artifact:${id})`} />);
    expect(html).not.toContain("<img"); expect(html).toContain(`/api/agents/root/artifacts/${id}/content`);
    expect(html).toContain('download=""'); expect(html).not.toContain(`/agents/root/artifacts?`);
    expect(artifactContentUrl("root/other", id)).toContain("root%2Fother");
  });
  it("shows queued calls and execution timings without queue wait timings", () => {
    const html = renderToStaticMarkup(<ToolActivity tools={[{ id: "waiting", name: "↳ indicator_list", status: "queued" }, { id: "completed", name: "↳ indicator_get", status: "ok", durationMs: 2, queuedMs: 35 }]} />);
    expect(html).toContain("indicator_list · queued"); expect(html).toContain("indicator_get · ok · 2ms"); expect(html).not.toContain("35ms"); expect(html).toContain("open=\"\"");
  });
  it("shows only execution time for inline tool calls", () => {
    const html = renderToStaticMarkup(<ToolCall tool={{ id: "call", name: "research_candles", status: "ok", durationMs: 202, queuedMs: 403 }} />);
    expect(html).toContain("202ms");
    expect(html).not.toContain("403ms");
    expect(html).not.toContain("queued");
  });
  it("shows nested failure summaries without artifact navigation", () => {
    const html = renderToStaticMarkup(<ToolActivity tools={[{ id: "call", name: "↳ indicator_run", status: "error", error: "Missing candles", arguments: '{"coin":"BTC"}', durationMs: 10 }]} />);
    expect(html).toContain("Missing candles"); expect(html).toContain("indicator_run"); expect(html).toContain("10ms"); expect(html).not.toContain("Open artifacts"); expect(html).not.toContain("/agents/root/artifacts");
  });
});

import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { ArtifactContent, artifactContentUrl } from "./artifact-content.js";
import { ToolActivity } from "./tool-activity.js";
const id = "00000000-0000-0000-0000-000000000001";
describe("safe artifact rendering", () => {
  it("escapes active content and embeds only resolved image attachments", () => {
    const html = renderToStaticMarkup(<ArtifactContent agentId="root" content={`<script>alert(1)</script>\n![Chart](artifact:${id})\n![External](https://example.test/tracker.png)`} attachments={[{ id, title: "Chart", mediaType: "image/png" }]} />);
    expect(html).not.toContain("<script>"); expect(html).toContain("&lt;script&gt;");
    expect(html).toContain(`/api/agents/root/artifacts/${id}/content?inline=1`);
    expect(html).not.toContain('src="https://example.test');
    expect(html).toContain('alt="Chart"');
  });
  it("renders unresolved references as artifact links, never external images or recursive HTML", () => {
    const html = renderToStaticMarkup(<ArtifactContent agentId="root" content={`![Missing](artifact:${id})`} />);
    expect(html).not.toContain("<img"); expect(html).toContain(`/agents/root/artifacts?artifact=${id}`);
    expect(artifactContentUrl("root/other", id)).toContain("root%2Fother");
  });
  it("shows queued calls and retained queue wait timings", () => {
    const html = renderToStaticMarkup(<ToolActivity agentId="root" tools={[{ id: "waiting", name: "↳ ta_rsi", status: "queued" }, { id: "completed", name: "↳ ta_ema", status: "ok", durationMs: 2, queuedMs: 35 }]} />);
    expect(html).toContain("ta_rsi · queued"); expect(html).toContain("queued 35ms"); expect(html).toContain("open=\"\"");
  });
  it("shows nested failure summaries and artifact navigation", () => {
    const html = renderToStaticMarkup(<ToolActivity agentId="root" tools={[{ id: "call", name: "↳ ta_macd", status: "error", error: "Missing candles", arguments: '{"coin":"BTC"}', durationMs: 10 }]} />);
    expect(html).toContain("Missing candles"); expect(html).toContain("ta_macd"); expect(html).toContain("10ms"); expect(html).toContain("/agents/root/artifacts");
  });
});

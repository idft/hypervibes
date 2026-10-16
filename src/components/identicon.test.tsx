import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Identicon } from "./identicon.js";

describe("wallet identicon", () => {
  it("renders a safe, deterministic SVG data URI per address", () => {
    const address = "0x9494488e0f29c3d705439eea285530aaabf01e69";
    const html = renderToStaticMarkup(<Identicon address={address} className="h-5 w-5 rounded-full" />);
    const uri = html.match(/data:image\/svg\+xml,[^"]+/)?.[0] ?? "";
    expect(uri).toContain("data:image/svg+xml,");
    expect(uri).not.toContain("<");
    expect(uri).toContain("hsl(");
    const same = renderToStaticMarkup(<Identicon address={address} />);
    const other = renderToStaticMarkup(<Identicon address="0x1234567890abcdef1234567890abcdef12345678" />);
    expect(same).toContain(uri);
    expect(other).not.toContain(uri);
  });
});
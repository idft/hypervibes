// @vitest-environment jsdom
import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

vi.mock("@tanstack/react-router", () => ({
  useRouter: () => ({ invalidate: vi.fn() }),
  Link: ({ to, params, hash, ...props }: ComponentProps<"a"> & { to: string; params?: { agentId: string }; hash?: string }) => <a href={`${to.replace("$agentId", params?.agentId ?? "")}${hash ? `#${hash}` : ""}`} {...props} />,
}));
vi.mock("../web/auth-client.js", () => ({ authClient: { signOut: vi.fn() } }));
import { Navbar as NavbarComponent } from "./navbar.js";
import { createTestQueryClient, withTestQueryClient } from "../test/query.js";
const Navbar = withTestQueryClient(NavbarComponent, createTestQueryClient);

describe("Navbar", () => {
  it("uses the horizontal wordmark as the accessible home link", () => {
    const container = document.createElement("div");
    container.innerHTML = renderToStaticMarkup(<Navbar walletAddress="0xfixture" />);
    const logo = container.querySelector('img[alt="HyperVibes"]')!;
    expect(logo.getAttribute("src")).toBe("/static/hypervibes.png");
    expect(logo.closest("a")?.getAttribute("href")).toBe("/agents");
    expect(logo.parentElement?.textContent).toBe("");
    expect(container.querySelector('img[src="/static/logo.png"]')).toBeNull();
  });

  it("links the amber trading-disabled warning beside the picker to the selected Agent's risk settings", () => {
    const container = document.createElement("div");
    container.innerHTML = renderToStaticMarkup(<Navbar walletAddress="0xfixture" agents={[{ id: "one", name: "Research", enabled: true, tradingEnabled: false }]} selectedAgentId="one" />);
    const picker = container.querySelector("[data-agent-selector]")!;
    const warning = picker.nextElementSibling!;
    expect(warning.textContent).toBe("Trading disabled");
    expect(warning.getAttribute("href")).toBe("/agents/one/settings#risk-management");
    expect(warning.classList.contains("text-amber-300")).toBe(true);
    expect(warning.classList.contains("cursor-pointer")).toBe(true);
    expect(warning.getAttribute("title")).toContain("Reduce-only orders and cancellation remain available");
  });

  it.each(["one", undefined, "unknown"])("omits the warning for enabled trading or no selected Agent (%s)", (selectedAgentId) => {
    const markup = renderToStaticMarkup(<Navbar walletAddress="0xfixture" agents={[{ id: "one", name: "Research", enabled: true, tradingEnabled: true }, { id: "two", name: "Paused", enabled: true, tradingEnabled: false }]} selectedAgentId={selectedAgentId} />);
    expect(markup).not.toContain('href="/agents/one/settings#risk-management"');
    expect(markup).not.toContain('href="/agents/two/settings#risk-management"');
  });

  it("places an accessible, icon-only Providers link in the right-side controls", () => {
    const container = document.createElement("div");
    container.innerHTML = renderToStaticMarkup(<Navbar walletAddress={`0x${"11".repeat(20)}`} />);
    const links = container.querySelectorAll('a[href="/providers"]');
    expect(links).toHaveLength(1);
    const link = links[0]!;
    expect(link.parentElement?.classList.contains("ml-auto")).toBe(true);
    expect(link.getAttribute("title")).toBe("Provider connections");
    expect(link.getAttribute("aria-label")).toBe("Provider connections");
    expect(link.textContent).toBe("");
    expect(link.querySelector('svg[aria-hidden="true"]')).not.toBeNull();
    expect(link.classList.contains("cursor-pointer")).toBe(true);
  });

  it("replaces the Logout text with an icon-only button that keeps the pending state", () => {
    const container = document.createElement("div");
    container.innerHTML = renderToStaticMarkup(<Navbar walletAddress={`0x${"11".repeat(20)}`} />);
    const buttons = [...container.querySelectorAll("button")];
    const logout = buttons.find((button) => button.getAttribute("aria-label") === "Logout");
    expect(logout).toBeDefined();
    expect(logout?.textContent).toBe("");
    expect(logout?.getAttribute("title")).toBe("Logout");
    expect(logout?.querySelector('svg[aria-hidden="true"]')).not.toBeNull();
    expect(logout?.classList.contains("cursor-pointer")).toBe(true);
    expect(logout?.hasAttribute("disabled")).toBe(false);
  });
});

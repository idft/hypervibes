// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentRow } from "../db/agent-schema.js";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, params, children, className, title, onClick, "aria-label": label, "aria-expanded": expanded, "aria-controls": controls }: ComponentProps<"a"> & { to: string; params?: { agentId: string }; activeOptions?: unknown; activeProps?: unknown }) => <a href={to.replace("$agentId", params?.agentId ?? "")} className={className} title={title} aria-label={label} aria-expanded={expanded} aria-controls={controls} onClick={(event) => { event.preventDefault(); onClick?.(event); }}>{children}</a>,
  Outlet: () => <p>Page content</p>,
}));
vi.mock("./use-indicator-catalog-events.js", () => ({ useIndicatorCatalogEvents: () => "Indicator updates disconnected. Latest versions may be stale; reconnecting…" }));
import { AgentLayout } from "./agent-layout.js";
const agent: AgentRow = { id: "fixture", name: "Research", userId: "owner", state: "ready", conversationId: 1, initialProvider: null, initialModelId: null, error: null, tradingAccountAddress: null, creationRequestId: "fixture", createdAt: 0, updatedAt: 0 };
const navCookie = () => document.cookie.split(/;\s*/).find((part) => part.startsWith("hv-agent-nav-expanded="));

describe("Agent navigation", () => {
  let root: Root;
  let container: HTMLDivElement;
  beforeEach(async () => {
    document.cookie = "hv-agent-nav-expanded=; Max-Age=0";
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(<AgentLayout agent={agent} initialExpanded={false} />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); document.cookie = "hv-agent-nav-expanded=; Max-Age=0"; });
  it("links the requested sections, defaults to dashboard and pins Settings below the main links", () => {
    const links = [...container.querySelectorAll("nav a")];
    expect(links.map((link) => link.getAttribute("aria-label"))).toEqual(["Dashboard", "Charts", "Transactions", "Chat", "Sub-Agents", "Memories", "Settings"]);
    expect(links.map((link) => link.getAttribute("href"))).toEqual(["/agents/fixture", "/agents/fixture/chart", "/agents/fixture/transactions", "/agents/fixture/chat", "/agents/fixture/sub-agents", "/agents/fixture/memories", "/agents/fixture/settings"]);
    expect(links[6]?.closest(".mt-auto")).not.toBeNull();
    const divider = container.querySelector("hr")!;
    expect(divider.previousElementSibling?.getAttribute("aria-label")).toBe("Transactions");
    expect(divider.nextElementSibling?.getAttribute("aria-label")).toBe("Chat");
    expect(container.textContent).not.toContain("Research");
    expect(container.textContent).not.toContain("← Agents");
    expect(container.querySelector("h1")).toBeNull();
    expect(container.textContent).not.toContain("Notifications");
    expect(container.textContent).not.toContain("Indicator updates disconnected");
  });
  it("nests Indicators under Charts", async () => {
    const charts = container.querySelector<HTMLAnchorElement>('a[aria-label="Charts"]')!;
    expect(charts.getAttribute("aria-expanded")).toBe("false");
    expect(container.querySelector('#agent-chart-sections a')).toBeNull();
    await act(async () => charts.click());
    expect(charts.getAttribute("aria-expanded")).toBe("true");
    const indicator = container.querySelector('#agent-chart-sections a')!;
    expect(indicator.getAttribute("aria-label")).toBe("Indicators");
    expect(indicator.getAttribute("href")).toBe("/agents/fixture/indicators");
  });
  it("links Memories to the Memories page and expands its Insights sub-item without Artifacts", async () => {
    const button = container.querySelector<HTMLAnchorElement>('a[aria-label="Memories"]')!;
    expect(button.getAttribute("href")).toBe("/agents/fixture/memories");
    expect(button.getAttribute("aria-expanded")).toBe("false");
    expect(button.classList.contains("cursor-pointer")).toBe(true);
    expect(container.querySelector('#agent-memory-sections a')).toBeNull();
    await act(async () => button.click());
    expect(button.getAttribute("aria-expanded")).toBe("true");
    const children = [...container.querySelectorAll('#agent-memory-sections a')];
    expect(children.map((link) => link.getAttribute("aria-label"))).toEqual(["Insights"]);
    expect(container.querySelector('a[aria-label="Artifacts"]')).toBeNull();
    expect(children.map((link) => link.getAttribute("href"))).toEqual(["/agents/fixture/memories"]);
    await act(async () => button.click());
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect(container.querySelectorAll('#agent-memory-sections a')).toHaveLength(1);
  });
  it("expands labels and persists the preference for the next server render", async () => {
    const button = container.querySelector<HTMLButtonElement>('button[aria-label="Expand agent navigation"]')!;
    expect(button.getAttribute("aria-expanded")).toBe("false");
    expect(button.classList.contains("cursor-pointer")).toBe(true);
    await act(async () => button.click());
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect(navCookie()).toBe("hv-agent-nav-expanded=true");
    await act(async () => root.unmount()); root = createRoot(container);
    await act(async () => root.render(<AgentLayout agent={agent} initialExpanded={true} />));
    expect(container.querySelector('button[aria-label="Collapse agent navigation"]')!.getAttribute("aria-expanded")).toBe("true");
    expect(container.querySelector('a[aria-label="Dashboard"]')!.textContent).toBe("Dashboard");
  });
});

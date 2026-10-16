// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentRow } from "../db/agent-schema.js";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, params, children, className, title, "aria-label": label }: ComponentProps<"a"> & { to: string; params?: { agentId: string }; activeOptions?: unknown; activeProps?: unknown }) => <a href={to.replace("$agentId", params?.agentId ?? "")} className={className} title={title} aria-label={label}>{children}</a>,
  Outlet: () => <p>Page content</p>,
}));
import { AgentLayout } from "./agent-layout.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const agent: AgentRow = { id: "fixture", name: "Research", userId: "owner", state: "ready", conversationId: 1, initialProvider: null, initialModelId: null, error: null, tradingAccountAddress: null, creationRequestId: "fixture", createdAt: 0, updatedAt: 0 };

describe("Agent navigation", () => {
  let root: Root;
  let container: HTMLDivElement;
  beforeEach(async () => {
    localStorage.clear();
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(<AgentLayout agent={agent} />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); localStorage.clear(); });
  it("links the requested sections, defaults to dashboard and pins Settings below the main links", () => {
    const links = [...container.querySelectorAll("nav a")];
    expect(links.map((link) => link.getAttribute("aria-label"))).toEqual(["Dashboard", "Chart", "Chat", "Transactions", "Memories", "Sub-agents", "Settings"]);
    expect(links.map((link) => link.getAttribute("href"))).toEqual(["/agents/fixture", "/agents/fixture/chart", "/agents/fixture/chat", "/agents/fixture/transactions", "/agents/fixture/memories", "/agents/fixture/sub-agents", "/agents/fixture/settings"]);
    expect(links[6]?.closest(".mt-auto")).not.toBeNull();
    expect(container.textContent).not.toContain("Research");
    expect(container.textContent).not.toContain("← Agents");
    expect(container.querySelector("h1")).toBeNull();
    expect(container.textContent).not.toContain("Notifications");
  });
  it("expands labels and restores the preference after remount", async () => {
    const button = container.querySelector("button")!;
    expect(button.getAttribute("aria-expanded")).toBe("false");
    expect(button.classList.contains("cursor-pointer")).toBe(true);
    await act(async () => button.click());
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect(localStorage.getItem("agent-navigation-expanded")).toBe("true");
    await act(async () => root.unmount()); root = createRoot(container);
    await act(async () => root.render(<AgentLayout agent={agent} />));
    expect(container.querySelector("button")!.getAttribute("aria-expanded")).toBe("true");
    expect(container.querySelector('a[aria-label="Dashboard"]')!.textContent).toBe("Dashboard");
  });
});

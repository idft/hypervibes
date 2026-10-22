// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const navigate = vi.hoisted(() => vi.fn());
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, params, onClick, ...props }: ComponentProps<"a"> & { to: string; params?: { agentId: string } }) => {
    const href = to.replace("$agentId", params?.agentId ?? "");
    return <a {...props} href={href} onClick={(event) => { event.preventDefault(); onClick?.(event); navigate(href); }} />;
  },
}));
import { AgentSwitcher } from "./agent-switcher.js";
const agents = [{ id: "one", name: "Research", enabled: true, tradingEnabled: true }, { id: "two", name: "Momentum", enabled: true, tradingEnabled: false }];

describe("Agent switcher", () => {
  let root: Root;
  let container: HTMLDivElement;
  const details = () => container.querySelector("details")!;
  beforeEach(async () => {
    navigate.mockClear();
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(<AgentSwitcher agents={agents} selectedAgentId="one" />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  it("shows the current Agent with a green dot and lists agents plus creation", () => {
    expect(container.querySelector("summary")!.textContent).toBe("Research");
    expect(container.querySelector("summary .bg-emerald-400")).not.toBeNull();
    expect([...container.querySelectorAll("a")].map((link) => link.getAttribute("href"))).toEqual(["/agents/one", "/agents/two", "/agents/new"]);
    expect(container.querySelector('[aria-current="page"]')!.textContent).toBe("Research");
  });
  it("uses amber for disabled trading in the menu and updates the selected dot when settings change", async () => {
    expect(container.querySelector('a[href="/agents/one"] [aria-label="Trading enabled"]')?.classList.contains("bg-emerald-400")).toBe(true);
    expect(container.querySelector('a[href="/agents/two"] [aria-label="Trading disabled"]')?.classList.contains("bg-amber-400")).toBe(true);
    await act(async () => root.render(<AgentSwitcher agents={agents} selectedAgentId="two" />));
    expect(container.querySelector('summary [aria-label="Trading disabled"]')?.classList.contains("bg-amber-400")).toBe(true);
    await act(async () => root.render(<AgentSwitcher agents={agents.map((agent) => ({ ...agent, tradingEnabled: true }))} selectedAgentId="two" />));
    expect(container.querySelector('summary [aria-label="Trading enabled"]')?.classList.contains("bg-emerald-400")).toBe(true);
  });
  it("uses yellow for disabled agents in both the picker and menu even when trading is enabled", async () => {
    await act(async () => root.render(<AgentSwitcher agents={agents.map((agent) => agent.id === "one" ? { ...agent, enabled: false } : agent)} selectedAgentId="one" />));
    for (const selector of ["summary", 'a[href="/agents/one"]']) {
      const dot = container.querySelector(`${selector} [aria-label="Agent disabled"]`)!;
      expect(dot.classList.contains("bg-amber-400")).toBe(true);
      expect(dot.getAttribute("title")).toBe("Agent disabled");
      expect(container.querySelector(`${selector} .bg-emerald-400`)).toBeNull();
    }
    await act(async () => root.render(<AgentSwitcher agents={agents} selectedAgentId="one" />));
    expect(container.querySelector('summary [aria-label="Trading enabled"]')?.classList.contains("bg-emerald-400")).toBe(true);
    expect(container.querySelector('a[href="/agents/one"] [aria-label="Trading enabled"]')?.classList.contains("bg-emerald-400")).toBe(true);
  });
  it("switches to the chosen Agent dashboard and closes the menu", async () => {
    details().open = true;
    await act(async () => container.querySelector<HTMLAnchorElement>('a[href="/agents/two"]')!.click());
    expect(navigate).toHaveBeenCalledWith("/agents/two");
    expect(details().open).toBe(false);
    await act(async () => root.render(<AgentSwitcher agents={agents} selectedAgentId="two" />));
    expect(container.querySelector("summary")!.textContent).toBe("Momentum");
  });
  it("dismisses on outside click, Escape, or current Agent changing", async () => {
    details().open = true;
    await act(async () => document.body.click());
    expect(details().open).toBe(false);
    details().open = true;
    await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(details().open).toBe(false);
    expect(document.activeElement).toBe(container.querySelector("summary"));
    details().open = true;
    await act(async () => root.render(<AgentSwitcher agents={agents} selectedAgentId="two" />));
    expect(details().open).toBe(false);
  });
  it("shows Agents without a selected dot outside Agent pages, and supports an empty list", async () => {
    await act(async () => root.render(<AgentSwitcher agents={[]} />));
    expect(container.querySelector("summary")!.textContent).toBe("Agents");
    expect(container.querySelector("summary .bg-emerald-400")).toBeNull();
    expect(container.querySelector("a")!.textContent).toBe("Create new agent");
  });
});

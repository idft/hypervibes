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
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const agents = [{ id: "one", name: "Research" }, { id: "two", name: "Momentum" }];

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

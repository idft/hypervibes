// @vitest-environment jsdom
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { SetupChecklist, type SetupState } from "./setup-checklist.js";

const initial: SetupState = { providerConnected: false, apiKeyApproved: false, builderFeeApproved: false };

function render(state: SetupState, agentCount = 0) {
  const container = document.createElement("div");
  container.innerHTML = renderToStaticMarkup(<SetupChecklist state={state} agentCount={agentCount} />);
  return container;
}

describe("Setup checklist", () => {
  it.each([
    { state: initial, label: "Connect at least 1 AI provider", href: "/providers", completed: 0 },
    { state: { ...initial, providerConnected: true }, label: "Connect to Hyperliquid", href: "/account", completed: 1 },
    { state: { ...initial, providerConnected: true, apiKeyApproved: true }, label: "Approve the Hyperliquid builder fee", href: "/account", completed: 2 },
    { state: { providerConnected: true, apiKeyApproved: true, builderFeeApproved: true }, label: "Create your first agent", href: "/agents/new", completed: 3 },
  ])("highlights $label as the next step", ({ state, label, href, completed }) => {
    const container = render(state);
    const current = container.querySelector('[aria-current="step"] a');
    expect(current?.textContent).toBe(label);
    expect(current?.getAttribute("href")).toBe(href);
    expect(container.querySelectorAll(".sr-only").length).toBe(4);
    expect([...container.querySelectorAll(".sr-only")].filter((element) => element.textContent === "Completed: ")).toHaveLength(completed);
    expect(container.querySelector("section")?.className).toContain("border-violet-800");
  });

  it("uses actual completion even when steps were done out of order", () => {
    const container = render({ ...initial, apiKeyApproved: true, builderFeeApproved: true });
    expect(container.querySelector('[aria-current="step"] a')?.getAttribute("href")).toBe("/providers");
    expect(container.textContent).toContain("Completed: Connect to Hyperliquid");
  });

  it("disappears once an agent exists", () => {
    expect(render(initial, 1).innerHTML).toBe("");
  });
});

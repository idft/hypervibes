// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { DashboardSubagent } from "../agents/subagent-dashboard.js";
import { AgentSetupChecklist } from "./agent-setup-checklist.js";

function worker(overrides: Partial<DashboardSubagent> = {}): DashboardSubagent {
  return { id: "worker", name: "Worker", enabled: false, schedule: "", nextRunAt: null, runs: [], ...overrides };
}

function render(tradingEnabled = false, subagents: DashboardSubagent[] = [], agentId = "agent") {
  const container = document.createElement("div");
  container.innerHTML = renderToStaticMarkup(<AgentSetupChecklist agentId={agentId} tradingEnabled={tradingEnabled} subagents={subagents} />);
  return container;
}

const runStatuses = ["queued", "running", "completed", "failed", "cancelled", "skipped"] as const;

describe("Agent setup checklist", () => {
  it.each([false, true])("checks all eligibility combinations with trading enabled: %s", (tradingEnabled) => {
    for (const enabled of [false, true]) {
      for (const hasRuns of [false, true]) {
        const subagent = worker({ enabled, runs: hasRuns ? [{ id: "run", status: "completed", createdAt: 1, result: "" }] : [] });
        expect(render(tradingEnabled, [subagent]).innerHTML !== "").toBe(!(tradingEnabled && enabled) && !hasRuns);
      }
    }
    expect(render(tradingEnabled).innerHTML !== "").toBe(true);
  });

  it("shows the checklist when all workers are disabled and have no runs", () => {
    const container = render(false, [worker(), worker({ id: "second" })]);
    expect(container.querySelector('section[aria-label="Agent setup"]')).not.toBeNull();
    expect(container.querySelector("h2")?.textContent).toBe("Finish setting up your agent");
    expect(container.querySelector("section")?.className).toContain("border-violet-800");
    expect(container.querySelectorAll("ol li")).toHaveLength(2);
    expect(container.querySelector('[aria-current="step"] a')?.textContent).toBe("Enable Trading");
  });

  it.each([false, true])("uses any enabled worker to complete the sub-agent step with trading: %s", (tradingEnabled) => {
    const container = render(tradingEnabled, [worker(), worker({ id: "second", enabled: true })]);
    if (tradingEnabled) {
      expect(container.innerHTML).toBe("");
    } else {
      expect(container.querySelector("li:last-child .sr-only")?.textContent).toBe("Completed: ");
      expect(container.querySelector('[aria-current="step"] a')?.textContent).toBe("Enable Trading");
    }
  });

  it.each([
    { trading: true, subagents: [], completed: "Enable Trading", next: "Create a new sub-agent" },
    { trading: true, subagents: [worker()], completed: "Enable Trading", next: "Enable sub-agents" },
    { trading: false, subagents: [worker({ enabled: true })], completed: "Enable sub-agents", next: "Enable Trading" },
  ])("shows partial completion and the next step: $next", ({ trading, subagents, completed, next }) => {
    const container = render(trading, subagents);
    const steps = [...container.querySelectorAll("li")];
    const completedStep = steps.find((step) => step.querySelector("a")?.textContent === completed);
    expect(completedStep?.querySelector('[aria-hidden="true"]')?.textContent).toBe("✓");
    expect(completedStep?.querySelector(".sr-only")?.textContent).toBe("Completed: ");
    expect(completedStep?.className).toBe("text-zinc-400");
    expect(completedStep?.hasAttribute("aria-current")).toBe(false);
    expect(container.querySelectorAll('[aria-current="step"]')).toHaveLength(1);
    const nextStep = container.querySelector('[aria-current="step"]');
    expect(nextStep?.querySelector("a")?.textContent).toBe(next);
    expect(nextStep?.querySelector(".sr-only")?.textContent).toBe("Next: ");
    expect(nextStep?.querySelector('[aria-hidden="true"]')?.textContent).not.toBe("✓");
    expect(nextStep?.className).toBe("font-semibold text-violet-100");
  });

  it("updates completion and visibility as props change on the same mounted checklist", () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    const update = (tradingEnabled: boolean, subagents: DashboardSubagent[]) => {
      act(() => root.render(<AgentSetupChecklist agentId="agent" tradingEnabled={tradingEnabled} subagents={subagents} />));
    };
    try {
      update(false, []);
      expect(container.querySelector('[aria-current="step"] a')?.textContent).toBe("Enable Trading");
      update(true, []);
      expect(container.querySelector('[aria-current="step"] a')?.textContent).toBe("Create a new sub-agent");
      update(true, [worker()]);
      expect(container.querySelector('[aria-current="step"] a')?.textContent).toBe("Enable sub-agents");
      update(true, [worker({ enabled: true })]);
      expect(container.innerHTML).toBe("");
      update(false, [worker({ enabled: true })]);
      expect(container.querySelector('[aria-current="step"] a')?.textContent).toBe("Enable Trading");
      update(false, [worker({ runs: [{ id: "run", status: "queued", createdAt: 1, result: "" }] })]);
      expect(container.innerHTML).toBe("");
    } finally {
      act(() => root.unmount());
    }
  });

  it.each(runStatuses)("hides the checklist for a %s run, even on a disabled worker", (status) => {
    for (const tradingEnabled of [false, true]) {
      for (const enabled of [false, true]) {
        expect(render(tradingEnabled, [worker(), worker({ id: "second", enabled, runs: [{ id: "run", status, createdAt: 1, result: "" }] })]).innerHTML).toBe("");
      }
    }
  });

  it.each(["agent", "agent /?#%&é"])("uses ordinary links with an encoded agent ID: %s", (agentId) => {
    const base = `/agents/${encodeURIComponent(agentId)}`;
    for (const subagents of [[], [worker()]]) {
      const container = render(false, subagents, agentId);
      const links = [...container.querySelectorAll("a")];
      expect(links.map((link) => [link.textContent, link.getAttribute("href")])).toEqual([
        ["Enable Trading", `${base}/settings#risk-management`],
        subagents.length > 0
          ? ["Enable sub-agents", `${base}/sub-agents`]
          : ["Create a new sub-agent", `${base}/sub-agents/new`],
      ]);
      for (const link of links) expect(link.classList.contains("cursor-pointer")).toBe(true);
      expect(container.querySelector("button")).toBeNull();
    }
  });
});

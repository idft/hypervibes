// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SubagentRow } from "../db/subagent-schema.js";
import { templateDraft } from "../agents/subagent-templates.js";
import { workerTriggers } from "../agents/worker-triggers.js";
const mocks = vi.hoisted(() => ({ invalidate: vi.fn(), importDefaultSubagents: vi.fn(), success: vi.fn() }));
vi.mock("../web/subagents.js", () => ({ importDefaultSubagents: mocks.importDefaultSubagents, saveSubagentSection: vi.fn() }));
vi.mock("sonner", () => ({ toast: { success: mocks.success } }));
vi.mock("@tanstack/react-router", () => ({ useRouter: () => ({ invalidate: mocks.invalidate }), Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a> }));
import { SubagentsPage } from "./subagents-page.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const worker = (template: string, id: string): SubagentRow => {
  const draft = templateDraft(template, { provider: "test", modelId: "test-model", thinkingLevel: "" });
  return { ...draft, triggers: workerTriggers(draft.triggers), id, agentId: "agent", kind: "worker", archived: false, conversationId: null, nextRunAt: null, templateOrigin: null, createdAt: 1, updatedAt: 1 };
};
const analyst = worker("basic-analysis", "worker:analyst");
const trader = worker("basic-trading", "worker:trader");
trader.triggers.researchSources = [analyst.id];
const reviewer = worker("basic-review", "worker:reviewer");

describe("default sub-agent import UI and connections", () => {
  let root: Root, container: HTMLDivElement;
  const click = (label: string) => act(async () => { [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === label)!.click(); });
  beforeEach(async () => {
    vi.resetAllMocks();
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    mocks.importDefaultSubagents.mockResolvedValue({ ok: true, data: [analyst, trader, reviewer] });
    await act(async () => root.render(<SubagentsPage agentId="agent" definitions={[analyst, trader, reviewer]} />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

  it("shows actual source/recipient connections and configured account triggers, including paused state", () => {
    expect(container.textContent).toContain("Research from: Technical Analyst (paused)");
    expect(container.textContent).toContain("Research delivered to: Trader (paused)");
    expect(container.textContent).toContain("Triggers: research, fills, order-status changes");
    expect(container.textContent).toContain("Reviews Agent-wide activity");
  });
  it("requires confirmation, explains independent paused copies, and refreshes after import", async () => {
    await click("Import default sub-agents");
    expect(mocks.importDefaultSubagents).not.toHaveBeenCalled();
    expect(container.textContent).toContain("Existing sub-agents are not reused or changed");
    await click("Cancel");
    expect(container.textContent).not.toContain("Import paused sub-agents");
    await click("Import default sub-agents");
    await click("Import paused sub-agents");
    expect(mocks.importDefaultSubagents).toHaveBeenCalledWith({ data: { agentId: "agent", requestId: expect.any(String) } });
    expect(mocks.invalidate).toHaveBeenCalledOnce();
    expect(mocks.success).toHaveBeenCalledWith("Default sub-agents imported. All start paused.");
    expect(container.textContent).not.toContain("Import paused sub-agents");
  });
  it("keeps the same request ID on failure and blocks duplicate in-flight submissions", async () => {
    let settle!: (value: { ok: false; error: string }) => void;
    mocks.importDefaultSubagents.mockReturnValueOnce(new Promise((resolve) => { settle = resolve; }));
    await click("Import default sub-agents");
    await click("Import paused sub-agents");
    expect(container.textContent).toContain("Importing…");
    await click("Importing…");
    expect(mocks.importDefaultSubagents).toHaveBeenCalledOnce();
    await act(async () => settle({ ok: false, error: "Temporary failure" }));
    expect(container.querySelector('[role="alert"]')?.textContent).toBe("Temporary failure");
    await click("Import paused sub-agents");
    expect(mocks.importDefaultSubagents.mock.calls[1]).toEqual(mocks.importDefaultSubagents.mock.calls[0]);
  });
});

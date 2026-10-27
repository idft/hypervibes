import type { DashboardSubagent } from "../agents/subagent-dashboard.js";

export function AgentSetupChecklist({ agentId, tradingEnabled, subagents }: { agentId: string; tradingEnabled: boolean; subagents: DashboardSubagent[] }) {
  const subagentEnabled = subagents.some((subagent) => subagent.enabled);
  if ((tradingEnabled && subagentEnabled) || subagents.some((subagent) => subagent.runs.length > 0)) return null;
  const base = `/agents/${encodeURIComponent(agentId)}`;
  const steps = [
    { label: "Enable Trading", href: `${base}/settings#risk-management`, complete: tradingEnabled },
    subagents.length > 0
      ? { label: "Enable sub-agents", href: `${base}/sub-agents`, complete: subagentEnabled }
      : { label: "Create a new sub-agent", href: `${base}/sub-agents/new`, complete: false },
  ];
  const next = steps.findIndex((step) => !step.complete);
  return (
    <section aria-label="Agent setup" className="shrink-0 rounded-xl border border-violet-800 bg-violet-950/30 p-4 text-sm text-violet-200">
      <h2 className="font-semibold">Finish setting up your agent</h2>
      <ol className="mt-3 flex flex-wrap gap-x-6 gap-y-3">
        {steps.map((step, index) => <li key={step.href} aria-current={index === next ? "step" : undefined} className={step.complete ? "text-zinc-400" : index === next ? "font-semibold text-violet-100" : "text-violet-300/70"}>
          <span aria-hidden="true" className="mr-2">{step.complete ? "✓" : `${index + 1}.`}</span>
          <span className="sr-only">{step.complete ? "Completed: " : index === next ? "Next: " : ""}</span>
          <a href={step.href} className="cursor-pointer underline underline-offset-4 hover:text-white">{step.label}</a>
        </li>)}
      </ol>
    </section>
  );
}

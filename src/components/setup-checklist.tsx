export interface SetupState {
  providerConnected: boolean;
  apiKeyApproved: boolean;
  builderFeeApproved: boolean;
}

export function SetupChecklist({ state, agentCount }: { state: SetupState; agentCount: number }) {
  if (agentCount > 0) return null;
  const steps = [
    { label: "Connect at least 1 AI provider", href: "/providers", complete: state.providerConnected },
    { label: "Connect to Hyperliquid", href: "/account", complete: state.apiKeyApproved },
    { label: "Approve the Hyperliquid builder fee", href: "/account", complete: state.builderFeeApproved },
    { label: "Create your first agent", href: "/agents/new", complete: false },
  ];
  const next = steps.findIndex((step) => !step.complete);
  return (
    <section aria-label="Initial setup" className="shrink-0 rounded-xl border border-violet-800 bg-violet-950/30 p-4 text-sm text-violet-200">
      <h2 className="font-semibold">Get started</h2>
      <ol className="mt-3 flex flex-wrap gap-x-6 gap-y-3">
        {steps.map((step, index) => <li key={step.href + step.label} aria-current={index === next ? "step" : undefined} className={step.complete ? "text-zinc-400" : index === next ? "font-semibold text-violet-100" : "text-violet-300/70"}>
          <span aria-hidden="true" className="mr-2">{step.complete ? "✓" : `${index + 1}.`}</span>
          <span className="sr-only">{step.complete ? "Completed: " : index === next ? "Next: " : ""}</span>
          <a href={step.href} className="cursor-pointer underline underline-offset-4 hover:text-white">{step.label}</a>
        </li>)}
      </ol>
    </section>
  );
}

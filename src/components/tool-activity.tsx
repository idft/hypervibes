import type { ToolActivity as Activity } from "../research/tool-activity.js";
export function ToolCall({ tool }: { tool: Activity }) {
  return <details className="mr-8 rounded-lg border border-zinc-800 px-3 py-2 text-xs">
    <summary className={`cursor-pointer ${tool.status === "error" ? "text-red-300" : "text-zinc-400"}`}>
      <span className="font-medium">{tool.name}</span> · {tool.status}{tool.durationMs !== undefined && ` · ${tool.durationMs}ms`}
    </summary>
    <div className="mt-3 space-y-2 border-t border-zinc-800 pt-3">
      {tool.arguments && <div><p className="text-zinc-500">Arguments</p><pre className="whitespace-pre-wrap break-words text-zinc-400">{tool.arguments}</pre></div>}
      {tool.error && <p className="whitespace-pre-wrap break-words text-red-300">{tool.error}</p>}
      {tool.output && <div><p className="text-zinc-500">Result</p><pre className="whitespace-pre-wrap break-words text-zinc-400">{tool.output}</pre></div>}
    </div>
  </details>;
}

export function CodeExecution({ tool, calls }: { tool: Activity; calls: Activity[] }) {
  const active = ["running", "pending", "queued"].includes(tool.status);
  const label = tool.status === "done" ? "Script completed" : tool.status === "error" ? "Script failed" : tool.status === "running" ? "Running script…" : tool.status === "pending" || tool.status === "queued" ? "Script queued" : `Script ${tool.status}`;
  return <section aria-label="Code execution" className="space-y-3 rounded-xl border border-violet-900/60 bg-violet-950/10 p-3 text-xs">
    <header className="flex flex-wrap items-center justify-between gap-2">
      <h4 className="font-medium text-violet-200">Code execution <span className="font-normal text-zinc-500">· codemode</span></h4>
      <span className={tool.status === "error" ? "text-red-300" : active ? "text-violet-300" : "text-zinc-400"}>{active && <span aria-hidden="true" className="mr-1 inline-block h-2 w-2 animate-pulse rounded-full bg-violet-400" />}{label}{tool.durationMs !== undefined && ` · ${tool.durationMs}ms`}</span>
    </header>
    <details><summary className="cursor-pointer text-zinc-400">Script and output</summary><div className="mt-2 space-y-2">
      {tool.arguments && <pre className="whitespace-pre-wrap break-words text-zinc-400">{tool.arguments}</pre>}
      {tool.error && <p className="whitespace-pre-wrap break-words text-red-300">{tool.error}</p>}
      {tool.output && <pre className="whitespace-pre-wrap break-words text-zinc-400">{tool.output}</pre>}
    </div></details>
    {calls.length > 0 && <div className="space-y-2 border-l border-violet-900/60 pl-3">{calls.map((call) => <ToolCall key={call.id} tool={call} />)}</div>}
  </section>;
}

export function ToolActivity({ tools = [] }: { tools?: Activity[] }) {
  if (!tools.length) return null;
  return <details open={tools.some((tool) => ["error", "running", "pending", "queued"].includes(tool.status))} className="space-y-2 rounded-xl border border-zinc-800 p-3"><summary className="cursor-pointer text-sm text-zinc-300">Tool activity · {tools.length} calls</summary><p className="text-xs text-zinc-500">Bounded summaries; raw datasets are omitted.</p>{tools.map((tool) => <div key={tool.id} className="space-y-1 border-t border-zinc-800 py-2 text-xs"><p className={tool.status === "error" ? "text-red-300" : "text-violet-300"}>{tool.name} · {tool.status}{tool.durationMs !== undefined && ` · ${tool.durationMs}ms`}</p>{tool.arguments && <pre className="whitespace-pre-wrap break-words text-zinc-500">{tool.arguments}</pre>}{tool.error && <p className="text-red-300">{tool.error}</p>}{tool.output && <pre className="whitespace-pre-wrap break-words text-zinc-400">{tool.output}</pre>}</div>)}</details>;
}

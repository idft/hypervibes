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

export function ToolActivity({ tools = [] }: { tools?: Activity[] }) {
  if (!tools.length) return null;
  return <details open={tools.some((tool) => ["error", "running", "pending", "queued"].includes(tool.status))} className="space-y-2 rounded-xl border border-zinc-800 p-3"><summary className="cursor-pointer text-sm text-zinc-300">Tool activity · {tools.length} calls</summary><p className="text-xs text-zinc-500">Bounded summaries; raw datasets are omitted.</p>{tools.map((tool) => <div key={tool.id} className="space-y-1 border-t border-zinc-800 py-2 text-xs"><p className={tool.status === "error" ? "text-red-300" : "text-violet-300"}>{tool.name} · {tool.status}{tool.durationMs !== undefined && ` · ${tool.durationMs}ms`}</p>{tool.arguments && <pre className="whitespace-pre-wrap break-words text-zinc-500">{tool.arguments}</pre>}{tool.error && <p className="text-red-300">{tool.error}</p>}{tool.output && <pre className="whitespace-pre-wrap break-words text-zinc-400">{tool.output}</pre>}</div>)}</details>;
}

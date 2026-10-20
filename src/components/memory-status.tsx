const statusColors: Record<string, string> = {
  fresh: "border-emerald-500/30 bg-emerald-500/10 text-emerald-300",
  stale: "border-yellow-500/30 bg-yellow-500/10 text-yellow-300",
  retracted: "border-zinc-600 bg-zinc-800/50 text-zinc-400",
  invalid_evidence: "border-red-500/30 bg-red-500/10 text-red-300",
  durable: "border-violet-500/30 bg-violet-500/10 text-violet-300",
};

export function MemoryStatus({ status }: { status: string }) {
  return <span className={`inline-block rounded-full border px-2 py-0.5 text-xs ${statusColors[status] ?? "border-zinc-700 text-zinc-400"}`}>{status}</span>;
}

export function MemoryBadge({ value }: { value: string }) {
  return <span className="inline-block max-w-full break-words rounded-full border border-zinc-700 bg-zinc-800/50 px-2 py-0.5 text-xs text-zinc-300">{value}</span>;
}

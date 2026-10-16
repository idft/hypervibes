export function AgentSectionPage({ title, description }: { title: string; description: string }) {
  return <section className="w-full flex-1 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6"><h2 className="text-lg font-medium text-white">{title}</h2><p className="mt-3 text-sm text-zinc-400">{description}</p><p className="mt-2 text-xs text-zinc-500">Not implemented yet.</p></section>;
}

import type { RuntimeStatus } from "../runtime.js";

export function AgentsPage({ status }: { status: RuntimeStatus }) {
  return (
    <div className="mx-auto max-w-[1400px]">
      <div role="status" className="mb-6 rounded-xl border border-amber-800/70 bg-amber-950/30 px-4 py-3 text-sm text-amber-200">
        TypeScript preview · SQLite {status.database}. Agent execution is not configured; no trading is enabled.
      </div>
      <header className="border-b border-zinc-800 pb-6">
        <h1 className="text-3xl font-semibold tracking-tight text-white">Agents</h1>
      </header>
      <section className="mt-6 flex min-h-72 flex-col items-center justify-center rounded-2xl border border-dashed border-zinc-800 bg-[#0d0d10] px-6 py-12 text-center">
        <h2 className="text-xl font-semibold text-white">No agents yet</h2>
        <p className="mt-2 text-sm text-zinc-400">Agent management will be implemented next.</p>
        <button type="button" disabled title="Agent creation is not implemented yet" className="mt-6 inline-flex cursor-not-allowed items-center gap-2 rounded-full bg-violet-600 px-4 py-2 text-sm font-medium text-white opacity-50">
          Create a new agent
        </button>
      </section>
    </div>
  );
}

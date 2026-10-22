import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { indicatorListQueryOptions } from "../web/indicator-queries.js";
import { configButton, configInput } from "./config-dialog.js";

export interface IndicatorSearch { query: string; kind: "all" | "builtin" | "custom"; offset: number }
export function AgentIndicatorsPage({ agentId, search, onSearch }: { agentId: string; search: IndicatorSearch; onSearch: (search: IndicatorSearch) => void }) {
  const catalog = useQuery(indicatorListQueryOptions(agentId, search));
  return <section className="w-full space-y-5 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
    <header className="flex flex-wrap items-center justify-between gap-4">
      <h2 className="text-lg font-medium text-white">Indicators</h2>
      <Link className="cursor-pointer rounded-full border border-violet-500 px-4 py-2 text-sm text-violet-300 hover:bg-violet-950" to="/agents/$agentId/indicators/$indicatorId" params={{ agentId, indicatorId: "new" }} search={{}}>New indicator</Link>
    </header>
    <div className="flex flex-wrap gap-3">
      <label className="min-w-0 flex-1 text-sm text-zinc-300">Search<input className={configInput} value={search.query} onChange={(e) => onSearch({ ...search, query: e.target.value, offset: 0 })} /></label>
      <label className="text-sm text-zinc-300">Kind<select className={`${configInput} cursor-pointer`} value={search.kind} onChange={(e) => onSearch({ ...search, kind: e.target.value as IndicatorSearch["kind"], offset: 0 })}><option value="all">All</option><option value="builtin">Built-in</option><option value="custom">Custom</option></select></label>
    </div>
    {catalog.isPending && <p role="status">Loading indicators…</p>}
    {catalog.error && <p role="alert" className="text-red-300">{catalog.error.message}</p>}
    <div className="overflow-x-auto rounded-xl border border-zinc-800">
      <table className="w-full text-left text-sm">
        <thead className="border-b border-zinc-800 text-zinc-400"><tr><th scope="col" className="px-4 py-3 font-medium">Name</th><th scope="col" className="px-4 py-3 font-medium">Kind</th><th scope="col" className="px-4 py-3 font-medium">Description</th></tr></thead>
        <tbody className="divide-y divide-zinc-800">{catalog.data?.indicators.map((entry) => <tr key={entry.id} className="hover:bg-zinc-900">
          <td className="px-4 py-3"><Link className="cursor-pointer font-medium text-white hover:text-violet-300" to="/agents/$agentId/indicators/$indicatorId" params={{ agentId, indicatorId: entry.id }} search={{}}>{entry.name}</Link></td>
          <td className="whitespace-nowrap px-4 py-3 text-zinc-400">{entry.builtIn ? "Built-in · read-only" : "Custom"}</td>
          <td className="px-4 py-3 text-zinc-400"><Link className="block cursor-pointer hover:text-white" to="/agents/$agentId/indicators/$indicatorId" params={{ agentId, indicatorId: entry.id }} search={{}}>{entry.description}</Link></td>
        </tr>)}</tbody>
      </table>
      {catalog.data?.indicators.length === 0 && <p className="p-4 text-sm text-zinc-400">No indicators found.</p>}
    </div>
    <div className="flex justify-end gap-3"><button className={configButton} disabled={!search.offset} onClick={() => onSearch({ ...search, offset: Math.max(0, search.offset - 20) })}>Previous</button><button className={configButton} disabled={catalog.data?.nextOffset == null} onClick={() => onSearch({ ...search, offset: catalog.data!.nextOffset! })}>Next</button></div>
  </section>;
}

import { Link } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { NotificationRecord } from "../notifications/contracts.js";
import { notificationKeys, notificationQueryOptions, notificationsQueryOptions } from "../web/notification-queries.js";
import { useAgentEvent, useAgentEventsStatus } from "./agent-events.js";

const button = "cursor-pointer rounded-full border border-zinc-700 px-4 py-2 text-sm text-zinc-300 hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-50";
const link = "cursor-pointer text-sm text-zinc-400 hover:text-white";
function timestamp(time: number) { return new Date(time).toISOString().replace("T", " ").replace(".000Z", " UTC").replace("Z", " UTC"); }
const deliveryLabels = { web_only: "Web only", pending: "Telegram pending", sent: "Sent to Telegram", failed: "Telegram failed", suppressed: "Telegram suppressed" };
function NotificationMetadata({ row }: { row: NotificationRecord }) {
  return <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-zinc-500">
    <span className={row.severity === "error" ? "text-red-300" : row.severity === "warning" ? "text-amber-300" : "text-zinc-400"}>{row.severity}</span>
    <time dateTime={new Date(row.createdAt).toISOString()}>{timestamp(row.createdAt)}</time><span>{row.source}</span><span>{deliveryLabels[row.delivery]}</span>
  </div>;
}
function useNotificationEvents(agentId: string) {
  const client = useQueryClient();
  useAgentEvent("notifications", () => { void client.invalidateQueries({ queryKey: notificationKeys.all(agentId) }); });
  const state = useAgentEventsStatus();
  return state.streamErrors.notifications || (state.interrupted ? state.controlMessage || "Notification updates disconnected. Reconnecting…" : "");
}
export function AgentNotificationsPage({ agentId, page, onPage }: { agentId: string; page: number; onPage: (page: number) => void }) {
  const query = useQuery(notificationsQueryOptions(agentId, page));
  const streamError = useNotificationEvents(agentId);
  const data = query.data;
  return <section className="w-full space-y-5 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
    <header className="flex flex-wrap items-center justify-between gap-4"><h2 className="text-lg font-medium text-white">Notifications</h2><Link className={button} to="/agents/$agentId/settings" params={{ agentId }} hash="notifications">Notification settings</Link></header>
    {streamError && <p role="status" className="text-sm text-amber-300">{streamError}</p>}
    {query.isPending && <p role="status" className="text-sm text-zinc-400">Loading notifications…</p>}
    {query.error && <p role="alert" className="text-sm text-red-300">{query.error.message} <button type="button" className={link} onClick={() => { void query.refetch(); }}>Retry</button></p>}
    {data && <>
      {!data.total ? <p className="text-sm text-zinc-500">No notifications yet. Notifications from this Agent and its account activity will appear here.</p> : <ul className="space-y-3">{data.items.map((row) => <li key={row.id} className="rounded-xl border border-zinc-800 p-4 hover:border-violet-700"><Link to="/agents/$agentId/notifications/$notificationId" params={{ agentId, notificationId: row.id }} className="block min-w-0 cursor-pointer space-y-2"><h3 className="break-words font-medium text-white">{row.title}</h3><NotificationMetadata row={row} /><p className="line-clamp-2 whitespace-pre-wrap break-words text-sm text-zinc-400">{row.body}</p></Link></li>)}</ul>}
      {data.total > 0 && <footer className="flex flex-wrap items-center justify-between gap-3 border-t border-zinc-800 pt-4"><p className="text-sm text-zinc-500">Showing {(data.page - 1) * 50 + 1}–{Math.min(data.page * 50, data.total)} of {data.total}</p><div className="flex items-center gap-3"><button type="button" className={button} disabled={data.page <= 1} onClick={() => onPage(data.page - 1)}>Previous</button><span className="text-sm text-zinc-400">Page {data.page} of {data.pages}</span><button type="button" className={button} disabled={data.page >= data.pages} onClick={() => onPage(data.page + 1)}>Next</button></div></footer>}
    </>}
  </section>;
}
export function AgentNotificationDetailPage({ agentId, notificationId }: { agentId: string; notificationId: string }) {
  const query = useQuery(notificationQueryOptions(agentId, notificationId));
  const streamError = useNotificationEvents(agentId), row = query.data;
  return <section className="w-full space-y-5 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
    <header><nav aria-label="Notification breadcrumb" className="flex min-w-0 flex-wrap items-center gap-2"><Link className={link} to="/agents/$agentId/notifications" params={{ agentId }} search={{ page: 1 }}><span aria-hidden="true">← </span>Notifications</Link><span aria-hidden="true" className="text-zinc-600">/</span><h2 aria-current="page" className="break-words text-lg font-medium text-white">{row?.title ?? "Notification"}</h2></nav></header>
    {streamError && <p role="status" className="text-sm text-amber-300">{streamError}</p>}
    {query.isPending && <p role="status" className="text-sm text-zinc-400">Loading notification…</p>}
    {query.error && <p role="alert" className="text-sm text-red-300">{query.error.message} <button type="button" className={link} onClick={() => { void query.refetch(); }}>Retry</button></p>}
    {row && <><NotificationMetadata row={row} /><p className="whitespace-pre-wrap break-words text-sm leading-relaxed text-zinc-300">{row.body}</p>
      <dl className="grid gap-4 border-t border-zinc-800 pt-4 text-sm sm:grid-cols-2"><div><dt className="text-zinc-500">Telegram delivery</dt><dd className="mt-1 text-zinc-300">{deliveryLabels[row.delivery]}{row.sentAt ? ` · ${timestamp(row.sentAt)}` : ""}{row.attempts ? ` · ${row.attempts} failed attempt(s)` : ""}</dd></div><div><dt className="text-zinc-500">Source</dt><dd className="mt-1 break-words text-zinc-300">{row.source}{row.orderRequestId && <p className="mt-1 break-all font-mono text-xs">Order request: {row.orderRequestId}</p>}</dd></div></dl>
      {row.error && <p role="status" className="text-sm text-amber-300">{row.error}</p>}
      {row.runId && row.subagentId ? <Link className="inline-block cursor-pointer text-sm text-violet-300 hover:underline" to="/agents/$agentId/sub-agents/$subagentId/runs/$runId" params={{ agentId, subagentId: row.subagentId, runId: row.runId }}>View originating run →</Link> : row.subagentId ? <Link className="inline-block cursor-pointer text-sm text-violet-300 hover:underline" to="/agents/$agentId/sub-agents/$subagentId" params={{ agentId, subagentId: row.subagentId }}>View source agent →</Link> : null}
    </>}
  </section>;
}

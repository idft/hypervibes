import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { toast } from "sonner";
import type { NotificationSettings } from "../notifications/contracts.js";
import { notificationKeys, notificationSettingsQueryOptions } from "../web/notification-queries.js";
import { saveNotificationSettings } from "../web/notifications.js";

const checkbox = "size-4 cursor-pointer accent-violet-500 disabled:cursor-not-allowed disabled:opacity-50";
function SettingsForm({ agentId, saved }: { agentId: string; saved: NotificationSettings }) {
  const client = useQueryClient();
  const [draft, setDraft] = useState(saved);
  const mutation = useMutation({
    mutationFn: (settings: NotificationSettings) => saveNotificationSettings({ data: { agentId, ...settings } }),
    onSuccess: async (settings) => {
      client.setQueryData(notificationSettingsQueryOptions(agentId).queryKey, settings);
      toast.success("Notification settings saved");
      await client.invalidateQueries({ queryKey: notificationKeys.all(agentId) });
    },
  });
  const dirty = draft.enabled !== saved.enabled || draft.submissions !== saved.submissions || draft.fills !== saved.fills;
  return <form className="mt-4 space-y-4" onSubmit={(event) => { event.preventDefault(); if (!mutation.isPending && dirty) mutation.mutate(draft); }}>
    <fieldset disabled={mutation.isPending} className="space-y-3 text-sm text-zinc-300">
      <label className="flex cursor-pointer items-center gap-3"><input type="checkbox" className={checkbox} checked={draft.enabled} onChange={(event) => setDraft((current) => ({ ...current, enabled: event.target.checked }))} />Enable Telegram notifications</label>
      <fieldset disabled={!draft.enabled} className="space-y-3 pl-7 disabled:opacity-50">
        <label className="flex cursor-pointer items-center gap-3"><input type="checkbox" className={checkbox} checked={draft.submissions} onChange={(event) => setDraft((current) => ({ ...current, submissions: event.target.checked }))} />Order submissions</label>
        <label className="flex cursor-pointer items-center gap-3"><input type="checkbox" className={checkbox} checked={draft.fills} onChange={(event) => setDraft((current) => ({ ...current, fills: event.target.checked }))} />Order fills (including external/manual trades)</label>
      </fieldset>
      {dirty && <button type="submit" disabled={mutation.isPending} className="cursor-pointer rounded-full bg-violet-600 px-4 py-2 text-sm text-white hover:bg-violet-500 disabled:cursor-wait disabled:opacity-50">{mutation.isPending ? "Saving…" : "Save Telegram notification settings"}</button>}
    </fieldset>
    {mutation.error && <p role="alert" className="text-sm text-red-300">{mutation.error.message}</p>}
  </form>;
}
export function NotificationSettingsSection({ agentId }: { agentId: string }) {
  const query = useQuery(notificationSettingsQueryOptions(agentId));
  return <section id="notifications" aria-label="Notifications" className="mt-6 scroll-mt-2 border-t border-zinc-800 pt-6">
    <h3 className="font-medium text-white">Notifications</h3>
    {query.isPending ? <p role="status" className="mt-4 text-sm text-zinc-400">Loading notification settings…</p> : query.data ? <SettingsForm key={agentId} agentId={agentId} saved={query.data} /> : null}
    {query.error && <p role="alert" className="mt-3 text-sm text-red-300">{query.error.message} <button type="button" className="cursor-pointer text-violet-300 hover:underline" onClick={() => { void query.refetch(); }}>Retry</button></p>}
  </section>;
}

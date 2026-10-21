import { queryOptions } from "@tanstack/react-query";
import { addJournalNote, getAgentJournal, getJournalAudit, getJournalNotes } from "./journal.js";

export type JournalView = "trades" | "activity";
export type JournalAudit = Awaited<ReturnType<typeof getJournalAudit>>;

/** The linked account syncs in the background, so the page refreshes while mounted. */
export function journalQueryOptions(agentId: string, view: JournalView, page: number) {
  return queryOptions({
    queryKey: ["journal", agentId, view, page],
    queryFn: () => getAgentJournal({ data: { id: agentId, view, page: String(page) } }),
    refetchInterval: (query) => query.state.data?.linked ? 5_000 : false,
  });
}

export function journalNotesQueryOptions(agentId: string, target: string) {
  return queryOptions({
    queryKey: ["journal-notes", agentId, target],
    queryFn: () => getJournalNotes({ data: { id: agentId, target } }),
  });
}

export function journalAuditQueryOptions(agentId: string, target: string) {
  return queryOptions({
    queryKey: ["journal-audit", agentId, target],
    queryFn: () => getJournalAudit({ data: { id: agentId, target } }),
  });
}

export function addJournalNoteMutation(agentId: string, target: string, body: string) {
  return { mutationFn: () => addJournalNote({ data: { id: agentId, target, body } }) };
}
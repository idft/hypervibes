import { journalDecisionAudit } from "../memory/decision-audit.js";
import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { agentAction, agentUser, field } from "./agents.js";

export const getAgentJournal = createServerFn({ method: "GET" })
  .validator((data: unknown) => {
    const view = field(data, "view"); const page = field(data, "page");
    if (view !== "trades" && view !== "activity") throw new Error("Invalid journal view");
    if (!/^\d{1,8}$/.test(page) || Number(page) < 1) throw new Error("Invalid journal page");
    return { id: field(data, "id"), view: view as "trades" | "activity", page: Number(page) };
  })
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentAction(() => context.runtime.hyperliquid.agentJournal(user.id, data.id, data.view, data.page));
  });
export const getJournalAudit = createServerFn({ method: "GET" }).validator((data: unknown) => ({ id: field(data, "id"), target: field(data, "target") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentAction(async () => { await context.runtime.agents.owned(user.id, data.id); return journalDecisionAudit(context.runtime.database, data.id, data.target); });
});
export const getJournalNotes = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ id: field(data, "id"), target: field(data, "target") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentAction(() => context.runtime.hyperliquid.journalNotes(user.id, data.id, data.target));
  });
export const addJournalNote = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id"), target: field(data, "target"), body: field(data, "body") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(() => context.runtime.hyperliquid.addJournalNote(user.id, data.id, data.target, data.body));
  });

import { useEffect, useRef, useState } from "react";
import type { AgentChat, ChatEntry } from "../agents/agent-service.js";
import { getAgentChatHistory } from "../web/agents.js";

/** Each mounted chat/settings page owns its stream and closes it on navigation. */
export function useAgentChat(initial: AgentChat) {
  const [chat, setChat] = useState(initial);
  const [connectionError, setConnectionError] = useState("");
  /** Transcript older than the streamed tail, oldest first. */
  const [older, setOlder] = useState<ChatEntry[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [historyDone, setHistoryDone] = useState(false);
  /** Exclusive upper entry ID for the next history page. */
  const before = useRef<number | undefined>(undefined);
  const chatRef = useRef(chat);
  const olderRef = useRef(older);
  const loadingRef = useRef(false);
  const doneRef = useRef(false);
  const id = initial.agent.id;
  useEffect(() => { chatRef.current = chat; }, [chat]);
  useEffect(() => { olderRef.current = older; }, [older]);
  useEffect(() => { loadingRef.current = historyLoading; }, [historyLoading]);
  useEffect(() => { doneRef.current = historyDone; }, [historyDone]);
  useEffect(() => {
    const source = new EventSource(`/api/agents/${id}/events`);
    const interrupted = (event: MessageEvent | undefined, fallback: string) => {
      setConnectionError(event ? (JSON.parse(event.data) as { message?: string }).message ?? fallback : fallback);
    };
    source.addEventListener("chat", (event) => {
      const next = JSON.parse((event as MessageEvent).data) as AgentChat;
      // A new context starts a clean history; pages loaded from the previous
      // context are no longer part of the visible transcript.
      if (next.messages.some((entry) => entry.marker === "reset")) {
        before.current = undefined;
        setOlder([]);
        setHistoryDone(false);
        setHistoryLoading(false);
      }
      chatRef.current = next;
      setChat(next);
      setConnectionError("");
    });
    source.addEventListener("unauthorized", () => { window.location.assign("/login"); });
    source.addEventListener("shutdown", () => interrupted(undefined, "Server is restarting. Reconnecting…"));
    source.addEventListener("unavailable", (event) => interrupted(event as MessageEvent, "Chat stream interrupted. Reconnecting…"));
    source.addEventListener("stream-error", (event) => interrupted(event as MessageEvent, "Conversation is too large to stream. Reload the page."));
    source.onerror = () => interrupted(undefined, "Connection interrupted. Reconnecting…");
    return () => { source.close(); };
  }, [id]);
  /** Prepends one page of transcript older than everything currently shown; returns whether a page was fetched. */
  async function loadOlder(): Promise<boolean> {
    if (loadingRef.current || doneRef.current) return false;
    loadingRef.current = true;
    setHistoryLoading(true);
    const result = await getAgentChatHistory({ data: { id, before: before.current } });
    if (!result.ok) {
      loadingRef.current = false;
      setHistoryLoading(false);
      setConnectionError(result.error);
      return false;
    }
    const known = new Set([...chatRef.current.messages, ...olderRef.current].map((entry) => entry.id));
    const fresh = result.data.messages.filter((entry) => !known.has(entry.id));
    if (fresh.length) setOlder((current) => [...fresh, ...current]);
    before.current = result.data.before;
    doneRef.current = !result.data.more;
    setHistoryDone(!result.data.more);
    loadingRef.current = false;
    setHistoryLoading(false);
    return true;
  }
  return { chat, messages: [...older, ...chat.messages], connectionError, historyLoading, historyDone, loadOlder };
}
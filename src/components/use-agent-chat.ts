import { useEffect, useId, useRef, useState } from "react";
import type { AgentChat, ChatEntry } from "../agents/agent-service.js";
import { useQueryClient } from "@tanstack/react-query";
import { agentHistoryQueryOptions } from "../web/agent-queries.js";
import { useAgentEvent, useAgentEventsStatus } from "./agent-events.js";

/** Each mounted chat/settings page owns its stream and closes it on navigation. */
export function useAgentChat(initial: AgentChat) {
  const client = useQueryClient();
  const historyId = useId();
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
  const lastReset = useRef(initial.messages.find((entry) => entry.marker === "reset")?.id);
  const historyVersion = useRef(0);
  const id = initial.agent.id;
  const { interrupted, controlMessage, streamErrors } = useAgentEventsStatus();
  useAgentEvent("chat", (event) => {
    const next = JSON.parse(event.data) as AgentChat;
    // Only a new reset boundary clears history, not every snapshot that
    // still contains the same marker. Invalidate pending old-context pages.
    const reset = next.messages.find((entry) => entry.marker === "reset")?.id;
    if (reset && reset !== lastReset.current) {
      lastReset.current = reset;
      historyVersion.current++;
      before.current = undefined;
      olderRef.current = [];
      loadingRef.current = false;
      doneRef.current = false;
      setOlder([]);
      setHistoryDone(false);
      setHistoryLoading(false);
    }
    chatRef.current = next;
    setChat(next);
    setConnectionError("");
  });
  useEffect(() => {
    const topicError = streamErrors["chat"];
    if (topicError) setConnectionError(topicError);
    else if (interrupted) setConnectionError(controlMessage || "Connection interrupted. Reconnecting…");
  }, [interrupted, controlMessage, streamErrors]);
  useEffect(() => { chatRef.current = chat; }, [chat]);
  useEffect(() => () => { historyVersion.current++; }, [id]);
  /** Prepends one page of transcript older than everything currently shown; returns whether a page was fetched. */
  async function loadOlder(): Promise<boolean> {
    if (loadingRef.current || doneRef.current) return false;
    loadingRef.current = true;
    setHistoryLoading(true);
    const version = historyVersion.current;
    try {
      const result = await client.fetchQuery(agentHistoryQueryOptions(id, before.current, `${historyId}:${version}`));
      if (version !== historyVersion.current) return false;
      const known = new Set([...chatRef.current.messages, ...olderRef.current].map((entry) => entry.id));
      const fresh = result.messages.filter((entry) => !known.has(entry.id));
      if (fresh.length) {
        olderRef.current = [...fresh, ...olderRef.current];
        setOlder(olderRef.current);
      }
      before.current = result.before;
      doneRef.current = !result.more;
      setHistoryDone(!result.more);
      setConnectionError("");
      return true;
    } catch {
      if (version === historyVersion.current) setConnectionError("Unable to load earlier messages. Please try again.");
      return false;
    } finally {
      if (version === historyVersion.current) {
        loadingRef.current = false;
        setHistoryLoading(false);
      }
    }
  }
  return { chat, messages: [...older, ...chat.messages], connectionError, historyLoading, historyDone, loadOlder };
}
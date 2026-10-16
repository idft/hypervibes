import { useEffect, useState } from "react";
import type { AgentChat } from "../agents/agent-service.js";

/** Each mounted chat/settings page owns its stream and closes it on navigation. */
export function useAgentChat(initial: AgentChat) {
  const [chat, setChat] = useState(initial);
  const [connectionError, setConnectionError] = useState("");
  const id = initial.agent.id;
  useEffect(() => {
    const source = new EventSource(`/api/agents/${id}/events`);
    const interrupted = (event: MessageEvent | undefined, fallback: string) => {
      setConnectionError(event ? (JSON.parse(event.data) as { message?: string }).message ?? fallback : fallback);
    };
    source.addEventListener("chat", (event) => {
      setChat(JSON.parse((event as MessageEvent).data) as AgentChat);
      setConnectionError("");
    });
    source.addEventListener("unauthorized", () => { window.location.assign("/login"); });
    source.addEventListener("shutdown", () => interrupted(undefined, "Server is restarting. Reconnecting…"));
    source.addEventListener("unavailable", (event) => interrupted(event as MessageEvent, "Chat stream interrupted. Reconnecting…"));
    source.addEventListener("stream-error", (event) => interrupted(event as MessageEvent, "Conversation is too large to stream. Reload the page."));
    source.onerror = () => interrupted(undefined, "Connection interrupted. Reconnecting…");
    return () => { source.close(); };
  }, [id]);
  return { chat, connectionError };
}

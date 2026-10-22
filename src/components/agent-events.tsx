import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import type { MultiplexTopic } from "../web/agent-events.js";

type ChartParams = { coin: string; interval: string };
type TopicHandler = (event: MessageEvent) => void;

interface AgentEventsState {
  interrupted: boolean;
  /** Last shutdown/unavailable payload message, if any; hooks fall back to their own text. */
  controlMessage: string;
  streamErrors: Partial<Record<MultiplexTopic, string>>;
  reconnect: () => void;
}

const AgentEventsContext = createContext<{
  subscribe: (topic: MultiplexTopic, handler: TopicHandler) => () => void;
  setChartParams: (params: ChartParams | null) => void;
  state: AgentEventsState;
} | null>(null);

function streamUrl(agentId: string, topics: MultiplexTopic[], chart: ChartParams | null): string {
  const params = new URLSearchParams({ topics: [...topics].sort().join(",") });
  if (topics.includes("chart") && chart) {
    params.set("coin", chart.coin);
    params.set("interval", chart.interval);
  }
  return `/api/agents/${encodeURIComponent(agentId)}/stream?${params}`;
}

/** Owns the single SSE connection for an agent page. Browser only; never opens during SSR. */
export function AgentEventsProvider({ agentId, children }: { agentId: string; children: ReactNode }) {
  const handlers = useRef(new Map<MultiplexTopic, Set<TopicHandler>>());
  const chartParams = useRef<ChartParams | null>(null);
  const scheduled = useRef(false);
  const source = useRef<EventSource | null>(null);
  const [connectionKey, setConnectionKey] = useState({ topics: "", coin: "", interval: "", epoch: 0 });
  const [interrupted, setInterrupted] = useState(false);
  const [controlMessage, setControlMessage] = useState("");
  const [streamErrors, setStreamErrors] = useState<Partial<Record<MultiplexTopic, string>>>({});

  const schedule = useCallback(() => {
    if (scheduled.current) return;
    scheduled.current = true;
    queueMicrotask(() => {
      scheduled.current = false;
      const topics = [...handlers.current.keys()].sort();
      const chart = chartParams.current;
      setConnectionKey((current) => {
        const next = { topics: topics.join(","), coin: chart?.coin ?? "", interval: chart?.interval ?? "", epoch: current.epoch };
        return next.topics === current.topics && next.coin === current.coin && next.interval === current.interval ? current : { ...next, epoch: current.epoch };
      });
    });
  }, []);

  const subscribe = useCallback((topic: MultiplexTopic, handler: TopicHandler) => {
    let set = handlers.current.get(topic);
    if (!set) { set = new Set(); handlers.current.set(topic, set); }
    set.add(handler);
    schedule();
    return () => {
      const current = handlers.current.get(topic);
      current?.delete(handler);
      if (current && current.size === 0) handlers.current.delete(topic);
      schedule();
    };
  }, [schedule]);

  const setChartParams = useCallback((params: ChartParams | null) => {
    const current = chartParams.current;
    if (current?.coin === params?.coin && current?.interval === params?.interval) return;
    chartParams.current = params;
    schedule();
  }, [schedule]);

  const reconnect = useCallback(() => {
    setConnectionKey((current) => ({ ...current, epoch: current.epoch + 1 }));
  }, []);

  useEffect(() => {
    if (typeof window === "undefined" || typeof EventSource === "undefined") return;
    if (!connectionKey.topics) return;
    const topics = connectionKey.topics.split(",") as MultiplexTopic[];
    const chart = connectionKey.coin && connectionKey.interval ? { coin: connectionKey.coin, interval: connectionKey.interval } : null;
    const url = streamUrl(agentId, topics, chart);
    const events = new EventSource(url);
    source.current = events;
    const clearError = () => { setInterrupted(false); };
    const markInterrupted = () => { setInterrupted(true); };
    const onControl = (event: Event) => {
      try {
        const payload = JSON.parse((event as MessageEvent).data) as { message?: string };
        if (payload.message) setControlMessage(payload.message);
      } catch { /* Keep the previous message; hooks fall back to their own text. */ }
      markInterrupted();
    };
    const dispatchers = new Map<MultiplexTopic, (event: Event) => void>();
    for (const topic of topics) {
      const dispatch = (event: Event) => {
        setStreamErrors((previous) => {
          if (!(topic in previous)) return previous;
          const next = { ...previous };
          delete next[topic];
          return next;
        });
        clearError();
        for (const handler of handlers.current.get(topic) ?? []) handler(event as MessageEvent);
      };
      dispatchers.set(topic, dispatch);
      events.addEventListener(topic, dispatch);
    }
    const onStreamError = (event: Event) => {
      try {
        const payload = JSON.parse((event as MessageEvent).data) as { topic?: MultiplexTopic; message?: string };
        if (payload.topic) {
          setStreamErrors((previous) => ({ ...previous, [payload.topic as MultiplexTopic]: payload.message ?? "Stream interrupted. Reconnecting…" }));
          return;
        }
      } catch { /* Fall through to a generic interruption. */ }
      markInterrupted();
    };
    events.addEventListener("stream-error", onStreamError);
    events.addEventListener("shutdown", onControl);
    events.addEventListener("unavailable", onControl);
    const onUnauthorized = () => { events.close(); window.location.assign("/login"); };
    events.addEventListener("unauthorized", onUnauthorized);
    events.onerror = markInterrupted;
    return () => {
      for (const [topic, dispatch] of dispatchers) events.removeEventListener(topic, dispatch);
      events.removeEventListener("stream-error", onStreamError);
      events.removeEventListener("shutdown", onControl);
      events.removeEventListener("unavailable", onControl);
      events.removeEventListener("unauthorized", onUnauthorized);
      events.onerror = null;
      events.close();
      if (source.current === events) source.current = null;
    };
  }, [agentId, connectionKey]);

  // Agent-scoped navigation unmounts the provider; ensure the socket closes promptly.
  useEffect(() => () => { source.current?.close(); source.current = null; }, []);

  return <AgentEventsContext.Provider value={{ subscribe, setChartParams, state: { interrupted, controlMessage, streamErrors, reconnect } }}>{children}</AgentEventsContext.Provider>;
}

function useAgentEventsContext() {
  const context = useContext(AgentEventsContext);
  if (!context) throw new Error("useAgentEvent must be used within AgentEventsProvider");
  return context;
}

/** Subscribes to one multiplexed topic; unregisters on unmount. Pass `enabled=false` to stay unsubscribed. */
export function useAgentEvent(topic: MultiplexTopic, handler: (event: MessageEvent) => void, enabled = true) {
  const { subscribe } = useAgentEventsContext();
  const handlerRef = useRef(handler);
  handlerRef.current = handler;
  useEffect(() => {
    if (!enabled) return;
    return subscribe(topic, (event) => handlerRef.current(event));
  }, [subscribe, topic, enabled]);
}

/** Subscribes to the chart topic with its coin/interval. Only one chart subscription is supported. */
export function useAgentChartEvents(coin: string, interval: string, handler: (event: MessageEvent) => void, enabled = true) {
  const { subscribe, setChartParams } = useAgentEventsContext();
  const handlerRef = useRef(handler);
  handlerRef.current = handler;
  useEffect(() => {
    if (!enabled) { setChartParams(null); return; }
    setChartParams({ coin, interval });
    return () => { setChartParams(null); };
  }, [setChartParams, coin, interval, enabled]);
  useEffect(() => {
    if (!enabled) return;
    return subscribe("chart", (event) => handlerRef.current(event));
  }, [subscribe, enabled]);
}

/** Shared connection status: whether the socket is interrupted, per-topic errors, and a manual reconnect. */
export function useAgentEventsStatus(): AgentEventsState {
  return useAgentEventsContext().state;
}

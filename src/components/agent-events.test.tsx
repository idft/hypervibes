// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentEventsProvider, useAgentChartEvents, useAgentEvent, useAgentEventsStatus } from "./agent-events.js";

class StreamSource extends EventTarget {
  static instances: StreamSource[] = [];
  close = vi.fn();
  onerror: (() => void) | null = null;
  constructor(readonly url: string) { super(); StreamSource.instances.push(this); }
  emit(event: string, data: unknown) { this.dispatchEvent(new MessageEvent(event, { data: JSON.stringify(data) })); }
}

const seen: Record<string, unknown[]> = {};
function ChatSub() { useAgentEvent("chat", (event) => { (seen.chat ??= []).push(JSON.parse(event.data)); }); return null; }
function DashboardSub() { useAgentEvent("dashboard", (event) => { (seen.dashboard ??= []).push(JSON.parse(event.data)); }); return null; }
function LayoutsSub() { useAgentEvent("chart-layouts", (event) => { (seen.layouts ??= []).push(JSON.parse(event.data)); }); return null; }
function IndicatorsSub() { useAgentEvent("indicators", (event) => { (seen.indicators ??= []).push(JSON.parse(event.data)); }); return null; }
function SubagentsSub() { useAgentEvent("subagents", (event) => { (seen.subagents ??= []).push(JSON.parse(event.data)); }); return null; }
function ChartSub({ coin, interval }: { coin: string; interval: string }) {
  useAgentChartEvents(coin, interval, (event) => { (seen.chart ??= []).push(JSON.parse(event.data)); });
  return null;
}
function StatusProbe({ onStatus }: { onStatus: (interrupted: boolean) => void }) {
  const { interrupted } = useAgentEventsStatus();
  onStatus(interrupted);
  return null;
}

describe("multiplexed agent events", () => {
  let root: Root, container: HTMLDivElement;
  const statuses: boolean[] = [];
  function tree(coin = "BTC", interval = "1h") {
    return <AgentEventsProvider agentId="agent">
      <ChatSub /><DashboardSub /><LayoutsSub /><IndicatorsSub /><SubagentsSub />
      <ChartSub coin={coin} interval={interval} />
      <StatusProbe onStatus={(interrupted) => { statuses.push(interrupted); }} />
    </AgentEventsProvider>;
  }
  const topicsOf = (url: string) => new URL(url, "http://localhost").searchParams;
  beforeEach(() => {
    for (const key of Object.keys(seen)) delete seen[key];
    statuses.length = 0;
    StreamSource.instances = [];
    vi.stubGlobal("EventSource", StreamSource);
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });

  it("multiplexes every agent topic over exactly one connection", async () => {
    await act(async () => root.render(tree()));
    await act(async () => {});
    expect(StreamSource.instances).toHaveLength(1);
    const params = topicsOf(StreamSource.instances[0]!.url);
    expect(params.get("topics")).toBe("chart,chart-layouts,chat,dashboard,indicators,subagents");
    expect(params.get("coin")).toBe("BTC");
    expect(params.get("interval")).toBe("1h");
    for (const entry of [["chart", { candles: [], status: "live" }], ["dashboard", { perpEquity: "1" }], ["chart-layouts", []], ["indicators", { revision: 3 }], ["subagents", []], ["chat", { messages: [] }]] as const) {
      await act(async () => StreamSource.instances[0]!.emit(entry[0], entry[1]));
    }
    expect(seen.chart).toEqual([{ candles: [], status: "live" }]);
    expect(seen.dashboard).toEqual([{ perpEquity: "1" }]);
    expect(seen.layouts).toEqual([[]]);
    expect(seen.indicators).toEqual([{ revision: 3 }]);
    expect(seen.subagents).toEqual([[]]);
    expect(seen.chat).toEqual([{ messages: [] }]);
    // A browser-level interruption surfaces until the next frame clears it.
    await act(async () => StreamSource.instances[0]!.onerror?.());
    expect(statuses.at(-1)).toBe(true);
    await act(async () => StreamSource.instances[0]!.emit("chat", { messages: [] }));
    expect(statuses.at(-1)).toBe(false);
  });

  it("reopens the single stream once when the chart selection changes", async () => {
    await act(async () => root.render(tree()));
    await act(async () => {});
    expect(StreamSource.instances).toHaveLength(1);
    await act(async () => root.render(tree("ETH", "5m")));
    await act(async () => {});
    expect(StreamSource.instances).toHaveLength(2);
    expect(StreamSource.instances[0]!.close).toHaveBeenCalledOnce();
    const params = topicsOf(StreamSource.instances[1]!.url);
    expect(params.get("topics")).toBe("chart,chart-layouts,chat,dashboard,indicators,subagents");
    expect(params.get("coin")).toBe("ETH");
    expect(params.get("interval")).toBe("5m");
    // Every topic keeps working on the reopened stream.
    await act(async () => StreamSource.instances[1]!.emit("chart", { candles: [], status: "live" }));
    expect(seen.chart).toEqual([{ candles: [], status: "live" }]);
  });
});

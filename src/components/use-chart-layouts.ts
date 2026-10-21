import { useEffect, useEffectEvent, useRef, useState } from "react";
import type { ChartEdit, ChartLayout } from "../charts/contracts.js";
import { changeChartLayout, createChartLayout, deleteChartLayout } from "../web/charts.js";
import { useMutation, useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import { chartLayoutsQueryOptions } from "../web/chart-queries.js";

/** SSE snapshots are authoritative, including reconnects, agent edits and other tabs. */
export function useChartLayouts(id: string, instruments: string[]) {
  const client = useQueryClient();
  const { data: layouts } = useSuspenseQuery(chartLayoutsQueryOptions(id));
  const [selected, setSelected] = useState(layouts[0]?.id ?? "");
  const mutation = useMutation({ mutationFn: (operation: () => Promise<ChartLayout | undefined>) => operation() });
  const pending = mutation.isPending;
  const error = mutation.error?.message;
  const [connectionError, setConnectionError] = useState("");
  const [ready, setReady] = useState(false);
  const busy = useRef(false), alive = useRef(true), creation = useRef(crypto.randomUUID()), streamVersion = useRef(0);
  const key = `hypervibes:chart-layout:${id}`;
  const restorePreference = useEffectEvent(() => {
    try { const saved = localStorage.getItem(key); if (saved && layouts.some((l) => l.id === saved)) setSelected(saved); } catch { /* Optional preference. */ }
  });
  useEffect(() => {
    alive.current = true;
    restorePreference();
    let active = true;
    const source = new EventSource(`/api/agents/${id}/chart/layouts/events`);
    const interrupted = (event: Event | undefined, fallback: string) => {
      if (active) setConnectionError(event ? (JSON.parse((event as MessageEvent).data) as { message?: string }).message ?? fallback : fallback);
    };
    source.addEventListener("chart-layouts", (event) => {
      if (!active) return;
      streamVersion.current++;
      const snapshot = JSON.parse((event as MessageEvent).data) as ChartLayout[];
      client.setQueryData(chartLayoutsQueryOptions(id).queryKey, (current = []) => {
        const known = new Map(current.map((layout) => [layout.id, layout]));
        // SSE owns collection membership, but an older frame must not undo an
        // already-committed mutation response received on the other connection.
        return snapshot.map((layout) => { const previous = known.get(layout.id); return previous && previous.revision > layout.revision ? previous : layout; });
      });
      setReady(true);
      setConnectionError("");
    });
    source.addEventListener("unauthorized", () => { if (active) { source.close(); window.location.assign("/login"); } });
    source.addEventListener("shutdown", () => interrupted(undefined, "Server is restarting. Reconnecting…"));
    source.addEventListener("unavailable", (event) => interrupted(event, "Chart layout stream interrupted. Reconnecting…"));
    source.addEventListener("stream-error", (event) => { interrupted(event, "Chart layouts are too large to stream. Reload the page."); source.close(); });
    source.onerror = () => interrupted(undefined, "Chart layout connection interrupted. Reconnecting…");
    return () => { active = false; alive.current = false; streamVersion.current++; source.close(); };
  }, [client, id]);
  async function run(operation: (before: number) => Promise<ChartLayout | undefined>) {
    if (busy.current) return undefined;
    busy.current = true;
    const before = streamVersion.current;
    try { return await mutation.mutateAsync(() => operation(before)); }
    catch { return undefined; /* Query retains the conflict; never blindly reapply an edit. */ }
    finally { busy.current = false; }
  }
  function choose(layoutId: string) {
    setSelected(layoutId);
    try { localStorage.setItem(key, layoutId); } catch { /* Optional preference. */ }
  }
  function accept(layout: ChartLayout, requireExisting = false) {
    if (!alive.current) return;
    client.setQueryData(chartLayoutsQueryOptions(id).queryKey, (current = []) => {
      const previous = current.find((item) => item.id === layout.id);
      if ((requireExisting && !previous) || (previous && previous.revision >= layout.revision)) return current;
      return [layout, ...current.filter((item) => item.id !== layout.id)];
    });
  }
  const layout = layouts.find((item) => item.id === selected) ?? layouts[0];
  async function create(name: string, duplicate = false, defaultId?: string) {
    const layoutId = defaultId ?? crypto.randomUUID();
    return run(async (before) => {
      const result = await createChartLayout({ data: { id, layoutId, settings: { name, ...(duplicate ? {} : { coin: instruments.includes("BTC") ? "BTC" : instruments[0] ?? "BTC", interval: "1h" }) }, ...(duplicate && layout ? { sourceId: layout.id } : {}) } });
      if (streamVersion.current === before) accept(result);
      if (alive.current) choose(result.id);
      return result;
    });
  }
  const initialize = useRef(false);
  const initializeLayout = useEffectEvent(() => { void create("My chart", false, creation.current); });
  useEffect(() => {
    if (!ready || initialize.current || !instruments.length) return;
    initialize.current = true;
    if (!layouts.length) initializeLayout();
  }, [ready, layouts.length, instruments.length]);
  async function change(action: "edit" | "undo" | "redo", edit?: ChartEdit) {
    if (!layout) return undefined;
    const requestId = crypto.randomUUID();
    return run(async (before) => {
      const result = await changeChartLayout({ data: { id, layoutId: layout.id, revision: layout.revision, requestId, action, edit } });
      // Compare revisions, not arrival order: a mid-request SSE frame may be
      // older than this committed response. Never resurrect a streamed deletion.
      accept(result, streamVersion.current !== before);
      return result;
    });
  }
  async function remove() {
    if (!layout) return;
    await run(async (before) => {
      await deleteChartLayout({ data: { id, layoutId: layout.id, revision: layout.revision } });
      if (alive.current) {
        if (streamVersion.current === before) client.setQueryData(chartLayoutsQueryOptions(id).queryKey, (current) => current?.filter((item) => item.id !== layout.id));
        setSelected("");
      }
      return undefined;
    });
  }
  return { layouts, layout, pending, error: error || connectionError, ready, choose, create, remove, change };
}

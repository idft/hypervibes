import { HyperliquidError } from "../hyperliquid/errors.js";
import type { ChartSelection, ChartSnapshot } from "../hyperliquid/chart-contracts.js";
import { AgentError, agentId } from "../agents/agent-input.js";
import type { AgentDashboard } from "../hyperliquid/agent-dashboard.js";
import type { DashboardSubagent } from "../agents/subagent-dashboard.js";
import type { RunHistory } from "../agents/subagent-history.js";
import type { RunDetail } from "../agents/subagent-service.js";
import type { AgentChat } from "../agents/agent-service.js";
import { currentUserSession } from "../auth-user.js";
import type { AppRuntime } from "../runtime.js";

const encoder = new TextEncoder();
const heartbeatMs = 15_000;
const maxFrameBytes = 2 * 1024 * 1024;
const frame = (event: string, data: unknown) => encoder.encode(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
export interface ChatEvent { event?: string; data: AgentChat }

/** HTTP subscriptions are app-owned, and closed before Fastify drains requests. */
export class AgentEventStreams {
  private stopping = false;
  private readonly connections = new Set<() => void>();
  private readonly tasks = new Set<Promise<void>>();
  private readonly users = new Map<string, number>();
  constructor(private readonly runtime: AppRuntime, private readonly kind: "chat" | "dashboard" | "chart" | "runs" | "run" | "subagents" = "chat") {}

  async open(request: Request, id: string, subagentId?: string, runId?: string): Promise<Response> {
    try {
      this.runtime.status();
      if (this.stopping) throw new AgentError("Server is shutting down", 503);
      if (request.method !== "GET") return new Response(null, { status: 405, headers: { Allow: "GET" } });
      if ((request.headers.get("origin") && request.headers.get("origin") !== this.runtime.origin)
        || request.headers.get("sec-fetch-site") === "cross-site") throw new AgentError("Invalid request origin", 403);
      id = agentId(id);
      const session = await currentUserSession(this.runtime, request.headers);
      if (!session) throw new AgentError("Sign in to view this agent", 401);
      await this.runtime.agents.owned(session.user.id, id);
      const params = new URL(request.url).searchParams;
      const selection = this.kind === "chart" ? await this.runtime.hyperliquid.charts.selection(params.get("coin") ?? "", params.get("interval") ?? "") : undefined;
      const history = this.kind === "runs" ? { subagentId: subagentId ?? "", page: Number(params.get("page") ?? "1") } : undefined;
      if (history) await this.runtime.agents.subagents.history(session.user.id, id, history.subagentId, history.page);
      const run = this.kind === "run" ? { subagentId: subagentId ?? "", runId: runId ?? "" } : undefined;
      if (run) await this.runtime.agents.subagents.ownedRun(session.user.id, id, run.subagentId, run.runId);
      if (this.stopping || request.signal.aborted) throw new AgentError("Connection closed", 503);
      const count = this.users.get(session.user.id) ?? 0;
      if (count >= 10) throw new AgentError("Too many open event connections", 429);
      this.users.set(session.user.id, count + 1);
      return this.connect(request, id, session.user.id, session.expiresAt, selection, history, run);
    } catch (error) {
      const expected = error instanceof AgentError || error instanceof HyperliquidError;
      const status = expected ? error.status : 503;
      const message = expected ? error.message : "Agent event stream is unavailable";
      return Response.json({ error: message }, { status, headers: { "Cache-Control": "no-store" } });
    }
  }

  private connect(request: Request, id: string, userId: string, expiresAt: Date, selection?: ChartSelection, history?: { subagentId: string; page: number }, run?: { subagentId: string; runId: string }): Response {
    const abort = new AbortController();
    let controller: ReadableStreamDefaultController<Uint8Array>;
    let pending: Uint8Array | undefined;
    let closed = false;
    let expiry = expiresAt.getTime();
    let heartbeat: ReturnType<typeof setInterval> | undefined;
    let deadline: ReturnType<typeof setTimeout> | undefined;
    let checking = false;
    let ticks = 0;
    const flush = () => {
      if (!closed && pending && (controller.desiredSize ?? 0) > 0) {
        const value = pending;
        pending = undefined;
        controller.enqueue(value);
      }
    };
    const finish = (event?: "unauthorized" | "unavailable" | "shutdown" | "stream-error", message?: string, cancelled = false) => {
      if (closed) return;
      closed = true;
      pending = undefined;
      clearInterval(heartbeat);
      clearTimeout(deadline);
      request.signal.removeEventListener("abort", disconnect);
      this.connections.delete(shutdown);
      const remaining = (this.users.get(userId) ?? 1) - 1;
      if (remaining > 0) this.users.set(userId, remaining);
      else this.users.delete(userId);
      abort.abort();
      try {
        if (!cancelled && event) controller.enqueue(frame(event, { message }));
        // Always close: a disconnected consumer still awaits a definite end.
        controller.close();
      } catch { /* The consumer already cancelled or the stream already ended. */ }
    };
    const disconnect = () => finish(undefined, undefined, true);
    const shutdown = () => finish("shutdown", "Server is restarting. Reconnecting…");
    const scheduleExpiry = () => {
      clearTimeout(deadline);
      const remaining = expiry - Date.now();
      if (remaining <= 0) { finish("unauthorized", "Session expired. Please sign in again."); return; }
      deadline = setTimeout(scheduleExpiry, Math.min(remaining, 2_147_483_647));
      deadline.unref();
    };
    const revalidate = async () => {
      if (closed || checking) return;
      checking = true;
      try {
        const session = await currentUserSession(this.runtime, request.headers);
        if (closed) return;
        if (!session || session.user.id !== userId) { finish("unauthorized", "Session ended. Please sign in again."); return; }
        await this.runtime.agents.owned(userId, id);
        if (closed) return;
        expiry = session.expiresAt.getTime();
        scheduleExpiry();
      } catch { finish("unavailable", "Unable to verify this session. Reconnecting…"); }
      finally { checking = false; }
    };
    const publish = (snapshot: AgentChat | AgentDashboard | ChartSnapshot | RunHistory | RunDetail | DashboardSubagent[]) => {
      if (closed) return;
      if (Date.now() >= expiry) { finish("unauthorized", "Session expired. Please sign in again."); return; }
      const value = frame(this.kind, snapshot);
      if (value.byteLength > maxFrameBytes) { finish("stream-error", "Snapshot is too large to stream. Reload the page to view it."); return; }
      // Keep one newest pending snapshot, not an unbounded queue of token events.
      pending = value;
      flush();
    };
    const stream = new ReadableStream<Uint8Array>({
      start(value) { controller = value; controller.enqueue(encoder.encode("retry: 3000\n\n")); },
      pull: flush,
      cancel: disconnect,
    }, { highWaterMark: 1 });
    this.connections.add(shutdown);
    request.signal.addEventListener("abort", disconnect, { once: true });
    scheduleExpiry();
    if (request.signal.aborted) disconnect();
    if (!closed) {
      heartbeat = setInterval(() => {
        if (closed) return;
        if ((controller.desiredSize ?? 0) > 0 && !pending) controller.enqueue(encoder.encode(": keepalive\n\n"));
        // Security checks only: no conversation reads or chat polling. Bound the
        // lifetime of a revoked session even when no new chat updates arrive.
        if (++ticks % 4 === 0) void revalidate();
      }, heartbeatMs);
      heartbeat.unref();
      const watching = Promise.resolve().then(() => this.kind === "chat"
        ? this.runtime.agents.watchChat(userId, id, abort.signal, publish)
        : this.kind === "subagents" ? this.runtime.agents.subagents.watchDashboard(userId, id, abort.signal, publish)
        : run ? this.runtime.agents.subagents.watchRun(userId, id, run.subagentId, run.runId, abort.signal, publish)
          : history ? this.runtime.agents.subagents.watchHistory(userId, id, history.subagentId, history.page, abort.signal, publish)
          : selection ? this.runtime.hyperliquid.charts.watch(selection, abort.signal, publish)
          : this.runtime.hyperliquid.watchDashboard(userId, id, abort.signal, publish));
      const task = watching
        .then(() => { if (!closed) shutdown(); })
        .catch(() => finish("unavailable", "Agent event stream interrupted. Reconnecting…"))
        .finally(() => { this.tasks.delete(task); });
      this.tasks.add(task);
    }
    return new Response(stream, { headers: {
      "Content-Type": "text/event-stream; charset=utf-8",
      "Cache-Control": "no-store",
      "X-Accel-Buffering": "no",
      "X-Content-Type-Options": "nosniff",
    } });
  }

  async close() {
    this.stopping = true;
    for (const finish of [...this.connections]) finish();
    await Promise.all(this.tasks);
  }
}

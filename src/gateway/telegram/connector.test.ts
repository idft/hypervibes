import type { EntryId, Submission } from "@earendil-works/pi-durable";
import { describe, expect, it, vi } from "vitest";
import type { AgentBridge } from "../../agents/agent-service.js";
import type { GatewayRow } from "../../db/gateway-schema.js";
import { forwardReply, submitChat, type BridgeMessage, type BridgeWorkspace, type GatewaySender } from "../handlers.js";
import { telegramConnector } from "./connector.js";

interface Recorded { sent: { chatId: number; text: string }[]; typed: number }

function sender(recording: Recorded): GatewaySender {
  return {
    async sendText(chatId, text) { recording.sent.push({ chatId, text }); },
    async sendTyping() { recording.typed++; },
  };
}

function gatewayRow(chatId: number | null): GatewayRow {
  return {
    agentId: "agent", type: "telegram", botTokenCiphertext: Buffer.alloc(1), botTokenHash: "hash", keyId: "primary",
    botUsername: "my_bot", chatId, chatUsername: chatId === null ? null : "operator", updatedAt: 0,
  };
}

function workspace(overrides: Partial<BridgeWorkspace> = {}): BridgeWorkspace {
  return {
    loadGateway: async () => gatewayRow(42),
    bindChat: async () => true,
    consumeLink: () => ({ status: "unknown" }),
    bridge: async () => { throw new Error("no bridge"); },
    ...overrides,
  };
}

const message = (text: string, chatId = 42): BridgeMessage => ({ chatId, text, fromUsername: "operator" });

async function receive(text: string, chatId = 42, ws: BridgeWorkspace = workspace()): Promise<Recorded> {
  const recording: Recorded = { sent: [], typed: 0 };
  await telegramConnector.handle("agent", message(text, chatId), ws, sender(recording));
  return recording;
}

describe("Telegram message routing", () => {
  it("ignores messages from chats that never completed the link flow", async () => {
    const recording = await receive("hello", 999);
    expect(recording.sent).toEqual([]);
  });

  it("guides a plain /start toward the web link flow before any binding exists", async () => {
    const recording = await receive("/start", 999);
    expect(recording.sent).toEqual([{ chatId: 999, text: "To link this chat, open your Agent's settings page and start a Telegram link." }]);
  });

  it("links the chat when a pending token is consumed and rejects it once", async () => {
    const consume = vi.fn(() => ({ status: "ok" as const, agentId: "agent" }));
    const bind = vi.fn(async () => true);
    const recording = await receive("/start 123e4567-e89b-42d2-a571-414174999999", 999, workspace({ consumeLink: consume, bindChat: bind }));
    expect(recording.sent).toEqual([{ chatId: 999, text: "Telegram chat linked. You can now use this chat with HyperVibes." }]);
    expect(bind).toHaveBeenCalledWith("agent", 999, "operator");
    expect(consume).toHaveBeenCalledWith("123e4567-e89b-42d2-a571-414174999999");
  });

  it("reports unknown, expired, mismatched and unconfigured links like the legacy gateway", async () => {
    const unknown = await receive("/start missing-token", 999, workspace({ consumeLink: () => ({ status: "unknown" }) }));
    expect(unknown.sent[0]?.text).toContain("unknown or has expired");
    const expired = await receive("/start expired-token", 999, workspace({ consumeLink: () => ({ status: "expired" }) }));
    expect(expired.sent[0]?.text).toContain("has expired");
    const mismatch = await receive("/start other-token", 999, workspace({ consumeLink: () => ({ status: "ok", agentId: "different-agent" }) }));
    expect(mismatch.sent[0]?.text).toContain("belongs to a different agent");
    const unconfigured = await receive("/start gone-token", 999, workspace({ consumeLink: () => ({ status: "ok", agentId: "agent" }), bindChat: async () => false }));
    expect(unconfigured.sent[0]?.text).toContain("no longer configured");
  });

  it("answers commands against the agent bridge", async () => {
    const bridge = {
      stop: vi.fn(async () => {}), reset: vi.fn(async () => {}), compact: vi.fn(async () => ({})),
    } as unknown as AgentBridge;
    const ws = workspace({ bridge: async () => bridge });
    expect((await receive("/stop", 42, ws)).sent).toEqual([{ chatId: 42, text: "Active turn stopped." }]);
    expect(bridge.stop).toHaveBeenCalled();
    expect((await receive("/reset", 42, ws)).sent).toEqual([{ chatId: 42, text: "Started a fresh conversation." }]);
    expect(bridge.reset).toHaveBeenCalled();
    expect((await receive("/compact", 42, ws)).sent).toEqual([{ chatId: 42, text: "Conversation compacted." }]);
    expect((await receive("/compact@my_bot", 42, ws)).sent).toEqual([{ chatId: 42, text: "Conversation compacted." }]);
    expect((await receive("/help", 42, ws)).sent[0]?.text).toContain("/reset");
    expect((await receive("/unknown", 42, ws)).sent[0]?.text).toContain("/reset");
  });

  it("surfaces command failures as operator-facing text", async () => {
    const ws = workspace({ bridge: async () => { throw new Error("Agent is unavailable. Retry initialization."); } });
    const recording = await receive("/stop", 42, ws);
    expect(recording.sent).toEqual([{ chatId: 42, text: "I couldn't process that message: Agent is unavailable. Retry initialization." }]);
  });

  it("submits plain text from the bound chat and forwards the settled reply", async () => {
    const submitted: string[] = [];
    const submission = {
      status: async () => ({ status: "placed" }),
      wait: async () => ({ status: "done", entry: 1 as EntryId, answer: 2 as EntryId }),
    } as unknown as Submission;
    const bridge = { submit: vi.fn(async (text: string) => { submitted.push(text); return submission; }), replyText: async () => ["All good."] } as unknown as AgentBridge;
    const recording = await receive("what is my position?", 42, workspace({ bridge: async () => bridge }));
    expect(submitted).toEqual(["what is my position?"]);
    await vi.waitFor(() => expect(recording.sent).toEqual([{ chatId: 42, text: "All good." }]));
    expect(recording.typed).toBeGreaterThan(0);
  });

  it("notifies when the turn is queued behind a running one", async () => {
    const submission = { status: async () => ({ status: "queued" }), wait: () => new Promise(() => {}) } as unknown as Submission;
    const bridge = { submit: async () => submission, replyText: async () => [] } as unknown as AgentBridge;
    const recording = await receive("and now?", 42, workspace({ bridge: async () => bridge, replyTimeoutMs: 10 }));
    await vi.waitFor(() => expect(recording.sent).toEqual([{ chatId: 42, text: "Message queued. It will send when the current turn finishes." }, { chatId: 42, text: expect.stringContaining("taking longer than expected") }]));
  });

  it("reports submission failures without throwing", async () => {
    const ws = workspace({ bridge: async () => { throw new Error("Agent message queue is full. Wait for a response."); } });
    const recording = await receive("hello", 42, ws);
    expect(recording.sent).toEqual([{ chatId: 42, text: "I couldn't process that message: Agent message queue is full. Wait for a response." }]);
  });
});

describe("generic turn forwarding", () => {
  it("cancels the submission waiter and typing request on endpoint shutdown without forwarding a late reply", async () => {
    const lifetime = new AbortController();
    let waiterSignal: AbortSignal | undefined;
    let finish!: (value: unknown) => void;
    const submission = { wait: (invocation: { abortSignal: AbortSignal }) => {
      waiterSignal = invocation.abortSignal;
      return new Promise((resolve) => { finish = resolve; });
    } } as unknown as Submission;
    let typingAborted = false;
    const send = vi.fn();
    const replyText = vi.fn(async () => ["Too late"]);
    const forwarding = forwardReply(workspace({ signal: lifetime.signal }), {
      sendText: send,
      sendTyping: async (_chatId, signal) => new Promise<void>((resolve) => {
        signal!.addEventListener("abort", () => { typingAborted = true; resolve(); }, { once: true });
      }),
    }, 42, { replyText } as unknown as AgentBridge, submission);
    expect(waiterSignal?.aborted).toBe(false);
    lifetime.abort();
    await forwarding;
    expect(waiterSignal?.aborted).toBe(true);
    expect(typingAborted).toBe(true);
    finish({ status: "done", entry: 1, answer: 2 });
    await Promise.resolve();
    expect(replyText).not.toHaveBeenCalled();
    expect(send).not.toHaveBeenCalled();
  });

  it("joins typing cancellation and leaves no timer when the reply finishes during a typing request", async () => {
    vi.useFakeTimers();
    try {
      let typingAborted = false;
      const submission = { wait: async () => ({ status: "done", entry: 1, answer: 2 }) } as unknown as Submission;
      await forwardReply(workspace(), {
        sendText: async () => {},
        sendTyping: async (_chatId, signal) => new Promise<void>((resolve) => {
          signal!.addEventListener("abort", () => { typingAborted = true; resolve(); }, { once: true });
        }),
      }, 42, { replyText: async () => ["Done"] } as unknown as AgentBridge, submission);
      expect(typingAborted).toBe(true);
      expect(vi.getTimerCount()).toBe(0);
    } finally { vi.useRealTimers(); }
  });

  it("does not submit or start typing for an already closed endpoint", async () => {
    const recording: Recorded = { sent: [], typed: 0 };
    const bridge = vi.fn();
    await submitChat("agent", 42, "hello", workspace({ bridge, signal: AbortSignal.abort() }), sender(recording));
    expect(bridge).not.toHaveBeenCalled();
    expect(recording).toEqual({ sent: [], typed: 0 });
  });

  it("forwards one chunk per reply and stops the typing indicator", async () => {
    const recording: Recorded = { sent: [], typed: 0 };
    const submission = { status: async () => ({ status: "placed" }), wait: async () => ({ status: "done", entry: 1 as EntryId, answer: 3 as EntryId }) } as unknown as Submission;
    const handle = { replyText: async () => ["first", "second"] } as unknown as AgentBridge;
    await forwardReply(workspace(), sender(recording), 42, handle, submission);
    expect(recording.sent).toEqual([{ chatId: 42, text: "first" }, { chatId: 42, text: "second" }]);
  });

  it("prefers the submission's provider detail over its machine reason", async () => {
    const recording: Recorded = { sent: [], typed: 0 };
    const submission = { status: async () => ({ status: "placed" }), wait: async () => ({ status: "unanswered", reason: "model_error", detail: "429: {\"message\":\"you (jkozan) have reached your session usage limit\"}" }) } as unknown as Submission;
    await forwardReply(workspace(), sender(recording), 42, {} as AgentBridge, submission);
    expect(recording.sent).toEqual([{ chatId: 42, text: "I couldn't process that message: 429: {\"message\":\"you (jkozan) have reached your session usage limit\"}" }]);
  });

  it("reports unanswered submissions with their reason", async () => {
    const recording: Recorded = { sent: [], typed: 0 };
    const submission = { status: async () => ({ status: "placed" }), wait: async () => ({ status: "unanswered", reason: "aborted: stopped" }) } as unknown as Submission;
    await forwardReply(workspace(), sender(recording), 42, {} as AgentBridge, submission);
    expect(recording.sent).toEqual([{ chatId: 42, text: "I couldn't process that message: aborted: stopped" }]);
  });

  it("stops forwarding after the timeout while the turn keeps running", async () => {
    const recording: Recorded = { sent: [], typed: 0 };
    const submission = { status: async () => ({ status: "placed" }), wait: () => new Promise(() => {}) } as unknown as Submission;
    await forwardReply(workspace({ replyTimeoutMs: 10 }), sender(recording), 42, {} as AgentBridge, submission);
    expect(recording.sent).toEqual([{ chatId: 42, text: "This response is taking longer than expected. The turn is still running; continue in the web dashboard." }]);
  });

  it("submits through the workspace bridge and ignores status read failures", async () => {
    const recording: Recorded = { sent: [], typed: 0 };
    const submission = { status: async () => { throw new Error("closed"); }, wait: async () => ({ status: "done", entry: 1 as EntryId, answer: 1 as EntryId }) } as unknown as Submission;
    const bridge = { submit: async () => submission, replyText: async () => [] } as unknown as AgentBridge;
    await submitChat("agent", 42, "hello", workspace({ bridge: async () => bridge }), sender(recording));
    await vi.waitFor(() => expect(recording.sent).toEqual([]));
  });
});
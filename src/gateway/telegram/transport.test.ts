import { beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ send: vi.fn(), typing: vi.fn(), init: vi.fn(), start: vi.fn(), stop: vi.fn(), on: vi.fn(), caught: vi.fn() }));
vi.mock("grammy", async (original) => {
  const module = await original<typeof import("grammy")>();
  return { ...module, Bot: class {
    api = { sendMessage: mocks.send, sendChatAction: mocks.typing };
    init = mocks.init;
    start = mocks.start;
    stop = mocks.stop;
    on = mocks.on;
    catch = mocks.caught;
  } };
});
import { GrammyError } from "grammy";
import { grammyEndpoint, sendTelegramNotification, TELEGRAM_MESSAGE_MAX_CHARS } from "./transport.js";

describe("Telegram notification transport", () => {
  beforeEach(() => vi.resetAllMocks());
  it("sends one plain-text message without starting polling and returns the Telegram ID", async () => {
    mocks.send.mockResolvedValue({ message_id: 123 });
    const signal = new AbortController().signal;
    expect(await sendTelegramNotification("test-only-token", 7, "A notification", signal)).toBe(123);
    expect(mocks.send).toHaveBeenCalledWith(7, "A notification", { link_preview_options: { is_disabled: true } }, signal);
  });
  it("honors retry_after and classifies permanent errors without leaking response payloads", async () => {
    mocks.send.mockRejectedValueOnce(new GrammyError("Failed", { ok: false, error_code: 429, description: "Too many requests", parameters: { retry_after: 30 } }, "sendMessage", {}));
    await expect(sendTelegramNotification("test-only-token", 7, "Message", new AbortController().signal)).rejects.toMatchObject({ permanent: false, retryAfterMs: 30000 });
    mocks.send.mockRejectedValueOnce(new GrammyError("Failed", { ok: false, error_code: 403, description: "Sensitive platform description" }, "sendMessage", {}));
    await expect(sendTelegramNotification("test-only-token", 7, "Message", new AbortController().signal)).rejects.toMatchObject({ permanent: true, message: "Telegram rejected delivery (403)" });
  });
  it("sanitizes network exceptions that may contain the token-bearing API URL", async () => {
    mocks.send.mockRejectedValue(new Error("Failed https://api.telegram.org/botSECRET/sendMessage"));
    await expect(sendTelegramNotification("test-only-token", 7, "Message", new AbortController().signal)).rejects.toMatchObject({ permanent: false, message: "Telegram is unreachable; retrying" });
  });
});

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => { resolve = done; });
  return { promise, resolve };
}

function pendingUntilAbort(signal: AbortSignal): Promise<never> {
  return new Promise((_, reject) => {
    if (signal.aborted) reject(signal.reason);
    else signal.addEventListener("abort", () => reject(signal.reason), { once: true });
  });
}

describe("Telegram endpoint lifecycle", () => {
  beforeEach(() => vi.resetAllMocks());

  it("cancels both outbound requests on stop and rejects later sends", async () => {
    mocks.send.mockImplementation((_chat, _text, _options, signal: AbortSignal) => pendingUntilAbort(signal));
    mocks.typing.mockImplementation((_chat, _action, _options, signal: AbortSignal) => pendingUntilAbort(signal));
    const endpoint = grammyEndpoint("test-only-token", vi.fn());
    const text = expect(endpoint.sendText(7, "hello", new AbortController().signal)).rejects.toThrow();
    const typing = expect(endpoint.sendTyping(7, new AbortController().signal)).rejects.toThrow();
    await endpoint.stop();
    await Promise.all([text, typing]);
    await expect(endpoint.sendText(7, "later")).rejects.toThrow();
    await expect(endpoint.sendTyping(7)).rejects.toThrow();
    expect(mocks.send).toHaveBeenCalledTimes(1);
    expect(mocks.typing).toHaveBeenCalledTimes(1);
  });

  it("combines caller cancellation with endpoint cancellation and stops further chunks", async () => {
    const endpoint = grammyEndpoint("test-only-token", vi.fn());
    const caller = new AbortController();
    mocks.send.mockImplementationOnce(() => { caller.abort(); return Promise.resolve({ message_id: 1 }); });
    await expect(endpoint.sendText(7, "x".repeat(TELEGRAM_MESSAGE_MAX_CHARS + 1), caller.signal)).rejects.toThrow();
    expect(mocks.send).toHaveBeenCalledTimes(1);
    mocks.typing.mockImplementation((_chat, _action, _options, signal: AbortSignal) => pendingUntilAbort(signal));
    const typingCaller = new AbortController();
    const typing = expect(endpoint.sendTyping(7, typingCaller.signal)).rejects.toThrow();
    typingCaller.abort();
    await typing;
    mocks.send.mockResolvedValue({ message_id: 2 });
    await endpoint.sendText(7, "unaffected");
    await endpoint.stop();
  });

  it("preserves ordered Unicode chunks and inbound message filtering", async () => {
    const received = vi.fn();
    const endpoint = grammyEndpoint("test-only-token", received);
    const text = "😀".repeat(TELEGRAM_MESSAGE_MAX_CHARS + 1);
    await endpoint.sendText(7, text);
    expect(mocks.send.mock.calls.map((call) => call[1])).toEqual(["😀".repeat(TELEGRAM_MESSAGE_MAX_CHARS), "😀"]);
    const handler = mocks.on.mock.calls[0]![1] as (ctx: unknown) => void;
    handler({ msg: { text: " hello " }, chat: { id: 7 }, from: { username: "user" } });
    handler({ msg: {}, chat: { id: 7 } });
    expect(received).toHaveBeenCalledExactlyOnceWith({ chatId: 7, text: "hello", fromUsername: "user" });
    await endpoint.stop();
  });

  it("cancels initialization and does not start polling after stop", async () => {
    mocks.init.mockImplementation((signal: AbortSignal) => pendingUntilAbort(signal));
    const endpoint = grammyEndpoint("test-only-token", vi.fn());
    const started = endpoint.start();
    await endpoint.stop();
    await started;
    expect(mocks.start).not.toHaveBeenCalled();
    await endpoint.start();
    expect(mocks.init).toHaveBeenCalledTimes(1);
  });

  it("joins polling after bot.stop resolves, without starting another polling task", async () => {
    const polling = deferred();
    mocks.start.mockReturnValue(polling.promise);
    const endpoint = grammyEndpoint("test-only-token", vi.fn());
    const onStarted = vi.fn();
    const started = endpoint.start(onStarted);
    expect(endpoint.start()).toBe(started);
    await vi.waitFor(() => expect(mocks.start).toHaveBeenCalledTimes(1));
    expect(mocks.start).toHaveBeenCalledWith({ allowed_updates: ["message"], onStart: onStarted });
    let stopped = false;
    const stop = endpoint.stop().then(() => { stopped = true; });
    await Promise.resolve();
    await Promise.resolve();
    expect(stopped).toBe(false);
    polling.resolve();
    await stop;
    await started;
    await endpoint.stop();
    expect(mocks.stop).toHaveBeenCalledTimes(1);
  });

  it("joins polling even when the final offset acknowledgement fails", async () => {
    const polling = deferred();
    mocks.start.mockReturnValue(polling.promise);
    mocks.stop.mockRejectedValue(new Error("acknowledgement failed"));
    const endpoint = grammyEndpoint("test-only-token", vi.fn());
    const started = endpoint.start();
    await vi.waitFor(() => expect(mocks.start).toHaveBeenCalledTimes(1));
    const stopped = expect(endpoint.stop()).rejects.toThrow("acknowledgement failed");
    polling.resolve();
    await stopped;
    await started;
  });

  it("retains start failures when the endpoint has not stopped", async () => {
    mocks.init.mockRejectedValue(new Error("invalid token"));
    const endpoint = grammyEndpoint("test-only-token", vi.fn());
    await expect(endpoint.start()).rejects.toThrow("invalid token");
    await endpoint.stop();
  });
});

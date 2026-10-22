import { beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ send: vi.fn() }));
vi.mock("grammy", async (original) => {
  const module = await original<typeof import("grammy")>();
  return { ...module, Bot: class { api = { sendMessage: mocks.send }; } };
});
import { GrammyError } from "grammy";
import { sendTelegramNotification } from "./transport.js";

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

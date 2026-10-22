import { Bot, GrammyError } from "grammy";
import { NotificationDeliveryError } from "../../notifications/delivery.js";
import { chunkText, type BridgeLogger, type BridgeMessage, type GatewayEndpoint } from "../handlers.js";

/** Telegram caps one message at 4096 characters. */
export const TELEGRAM_MESSAGE_MAX_CHARS = 4096;

/** Validates a bot token with Telegram and returns the bot's own username. */
export async function fetchBotUsername(token: string): Promise<string> {
  const bot = new Bot(token);
  await bot.init();
  return bot.botInfo.username;
}

/** Uses the existing bot's HTTP API without starting another polling endpoint. */
export async function sendTelegramNotification(token: string, chatId: number, text: string, signal: AbortSignal): Promise<number> {
  try {
    const bot = new Bot(token, { client: { timeoutSeconds: 15 } });
    // grammY's Node declarations use abort-controller's older signal type;
    // its HTTP client accepts the native signal's standard abort interface.
    const cancellation = signal as unknown as NonNullable<Parameters<typeof bot.api.sendMessage>[3]>;
    return (await bot.api.sendMessage(chatId, text, { link_preview_options: { is_disabled: true } }, cancellation)).message_id;
  } catch (error) {
    if (error instanceof GrammyError) {
      if (error.error_code === 429) throw new NotificationDeliveryError("Telegram rate limit; retrying", false, (error.parameters.retry_after ?? 1) * 1000);
      throw new NotificationDeliveryError(`Telegram rejected delivery (${error.error_code})`, error.error_code >= 400 && error.error_code < 500);
    }
    throw new NotificationDeliveryError("Telegram is unreachable; retrying");
  }
}

/** One grammY long-polling bot: routes messages and sends on behalf of the bridge. */
export function grammyEndpoint(token: string, onMessage: (message: BridgeMessage) => void, logger?: BridgeLogger): GatewayEndpoint {
  const bot = new Bot(token);
  bot.on("message", (ctx) => {
    const text = ctx.msg.text?.trim();
    const chatId = ctx.chat?.id;
    if (text && chatId !== undefined) onMessage({ chatId, text, ...(ctx.from?.username ? { fromUsername: ctx.from.username } : {}) });
  });
  // One failing update must not stop polling for the whole bot.
  bot.catch((error) => logger?.warn("Telegram update handling failed", error.error));
  return {
    async start(onStarted) { await bot.start({ allowed_updates: ["message"], ...(onStarted ? { onStart: onStarted } : {}) }); },
    async stop() { await bot.stop(); },
    async sendText(chatId: number, text: string) {
      for (const chunk of chunkText(text, TELEGRAM_MESSAGE_MAX_CHARS)) await bot.api.sendMessage(chatId, chunk);
    },
    async sendTyping(chatId: number) { await bot.api.sendChatAction(chatId, "typing"); },
  };
}
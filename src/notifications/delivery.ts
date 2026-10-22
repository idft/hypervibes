export class NotificationDeliveryError extends Error {
  constructor(message: string, readonly permanent = false, readonly retryAfterMs?: number, readonly suppressed = false) { super(message); }
}
export interface NotificationTarget { agentId: string; chatId: number; botTokenHash: string }
/** Sends exactly one bounded chunk; the caller persists its Telegram message ID. */
export type NotificationSender = (target: NotificationTarget, text: string, signal: AbortSignal) => Promise<number>;

# Notifications

Open Notifications inside an agent to read its messages, order activity, and
warnings about assignments discarded before starting or waiting on paused workers.
These system warnings remain in the app even when Telegram notifications are off.
Expired assignments are not retried; inspect the current account before starting
new work.

## Read notifications

- Browse messages with their time, source, and severity.
- Open a notification to read the full message.
- Follow a link to the originating sub-agent or run when available.
- Check Telegram delivery status and any delivery error.

## Choose what you receive

Open Settings, then Telegram, to:

- Enable or disable Telegram notifications.
- Include order submissions.
- Include order fills, including external or manual trades on the linked account.

Connect the [Telegram bridge](Telegram.md) to receive notifications in its linked
chat. The notification page shows whether a message is web-only, pending, sent,
failed, or suppressed for Telegram.

An order submission notification is not a fill confirmation. Check fills and
the current account state before assuming a trade completed.

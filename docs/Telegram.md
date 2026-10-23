# Telegram bridge

The Telegram bridge connects an agent's main conversation to a Telegram chat.
Messages from the linked chat go to the same conversation you use in the app.

## Connect Telegram

1. Create a bot with [@BotFather](https://t.me/BotFather) in Telegram.
2. Open the agent's Settings and choose Connect Telegram.
3. Paste the bot token and add it. Each bot can serve only one agent.
4. Choose Link Telegram chat, then Open Telegram.
5. Send the prefilled `/start` command to finish linking.

Keep the bot token private. Only link a chat whose participants you trust to
send requests to your agent. Its enabled tools and trading permissions also
apply to requests from Telegram.

## Chat and commands

- Send text messages and receive the agent's replies.
- Use `/stop` to stop the active response.
- Use `/compact` to summarize the conversation context.
- Use `/reset` to start a fresh conversation context.
- Use `/help` to see the available commands.

Stopping a response does not undo completed actions or exchange orders.

## Notifications and disconnection

- Enable [notifications](Notifications.md) to receive agent messages and selected order activity in the linked chat.
- Check Telegram delivery status on the app's Notifications page.
- Choose Disconnect Telegram in Settings to remove the bridge.

HyperVibes must stay running and connected to the internet for the bridge to work.

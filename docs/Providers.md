---
slug: /providers
---

# AI providers

Open Providers to connect the AI accounts your agents will use.

- Search the provider list.
- Connect with an API key or the provider's sign-in flow, where supported.
- Check connection status and available models.
- Disconnect a provider you no longer want to use.

Provider credentials are encrypted and private to your app user.
Never paste API keys into agent chat or instructions.

For OpenAI OAuth, choose **Browser (default)** or **Device code**.
Browser login opens the provider sign-in page in a new tab automatically.
If your browser blocks the tab, use **Open provider sign-in** in the dialog.
For OpenAI Codex on a remote server, choose **Device code**. Browser login
redirects to localhost on your browser's machine; alternatively, paste the full
redirect URL into the sign-in dialog to finish.

## Choose models

- Choose a model when creating an agent.
- Change the main conversation's model in Chat when it is idle.
- Choose a model for each sub-agent in its configuration.
- Set the thinking level when the selected model supports it.

Model calls use your connected provider account and may incur charges.
Disconnecting a provider can prevent agents using its models from running.
Reconnect it or select another available model.

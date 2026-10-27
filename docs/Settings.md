# Settings

Settings controls the selected agent's trading permissions, notifications,
and Telegram connection.

## Agent execution

Use **Disable agent** to stop active runs and prevent new work, or **Enable
agent** to resume. Orders, positions, and trading risk settings remain unchanged.
See [Emergency stop](Agents.md#emergency-stop-and-re-enable).

Lifecycle status such as `ready` describes initialization, not enablement.
Trading permissions are controlled separately by the risk settings below.

## Risk management

- Enable Trading allows the agent and its workers to open trading exposure. It starts off.
- Allowed instruments restricts which markets can receive new exposure. No restriction allows all supported instruments.
- Max order notional caps the value of an opening order in USDC.
- Max total exposure caps projected position value plus pending entries in USDC.
- Choose No limit to leave a value uncapped, or enter a limit and save it.

Notional means position value, not the amount of margin used. These controls
restrict new exposure. They are not stop-losses and do not guarantee a maximum
loss as prices move.

Turning trading off does not close positions or cancel existing orders.
Agents with order tools can still submit orders that only reduce exposure or
manage protection. Disable order tools for research-only work.

Written strategy instructions are not enforced risk limits.

## Connections and messages

- Check the linked trading account and agent status.
- Choose which [notifications](Notifications.md) to receive in Telegram.
- Connect or disconnect the [Telegram bridge](Telegram.md).

Manage wallet approvals on [Account](Hyperliquid.md) and AI connections on
[Providers](Providers.md).

## Delete an agent

Type the agent's name to confirm permanent deletion. Deletion stops its workers,
removes its configuration and app history, and releases its account for another
agent to use.

Existing Hyperliquid positions and orders remain unchanged. Manage them on the
exchange before deleting an agent if you no longer want that exposure.

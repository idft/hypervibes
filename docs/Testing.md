---
slug: /development/testing
---

# Check your setup

Before allowing live orders, check the account, model, and worker setup.

## Account

- Open Account and check that your Hyperliquid account is active.
- Check that the trading API key is approved and has not expired.
- Review the builder-fee approval.
- Confirm the agent has the intended trading account.

## Models and workers

- Check the connection on Providers if a model is unavailable.
- Send a simple message in Chat and confirm the agent replies.
- Run a research worker manually and inspect its result before enabling a schedule.
- Check the worker's tools, instructions, and timeout if it cannot complete a task.
- Check that a worker is enabled and not archived if automatic assignments do not run.

## Trading and data

- Keep trading disabled while checking research-only tasks.
- Review allowed instruments and exposure limits before enabling trading.
- Check Dashboard connection warnings and Transactions sync warnings before trusting displayed totals.
- Check Notifications for order and Telegram delivery errors.

If an order's outcome is unclear, check the actual orders and positions on
Hyperliquid before requesting another order. Stopping a run does not undo
exchange actions.

Run Vibetrading trading execution for this agent.

- Follow the Trading strategy and the job prompt's Instructions for execution,
  sizing, entries, stops, and take-profits.
- If real trading tools are enabled, submit or cancel orders only through
  the `vibetrading` MCP trading tools (`vibetrading_submit_orders`,
  `vibetrading_cancel_orders`, `vibetrading_cancel_all_orders`). Never sign
  orders directly or request private keys.
- Do not call Vibetrading HTTP APIs directly, and do not author Python
  scripts to reach the backend.

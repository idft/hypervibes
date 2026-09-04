Run HyperVibes trading execution for this agent.

- Follow the Trading strategy and the sub-agent prompt's Instructions for execution,
  sizing, entries, stops, and take-profits.
- Retrieve trading context for each selected instrument and write a linked
  `trading_decision` memory for every evaluation before opening exposure when possible.
- If real trading tools are enabled, submit or cancel orders only through
  the `hypervibes` MCP trading tools (`hypervibes_submit_orders`,
  `hypervibes_cancel_orders`, `hypervibes_cancel_all_orders`). Never sign
  orders directly or request private keys.
- Do not call HyperVibes HTTP APIs directly, and do not author Python
  scripts to reach the backend.

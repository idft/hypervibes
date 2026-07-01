Run Vibetrading trading execution for this agent.

- Read the latest fresh `market_analysis` memory for each selected symbol with `vibetrading_get_market_analysis(symbol)`.
- Do not open new exposure when no fresh market analysis exists.
- Do not fall back to raw timeframe `analysis` memories for execution decisions unless the operator prompt explicitly asks for diagnostics.
- If real trading tools are enabled, submit or cancel orders only through
  the `vibetrading` MCP trading tools (`vibetrading_submit_orders`,
  `vibetrading_cancel_orders`, `vibetrading_cancel_all_orders`). Never sign
  orders directly or request private keys.
- Do not call Vibetrading HTTP APIs directly, and do not author Python
  scripts to reach the backend.

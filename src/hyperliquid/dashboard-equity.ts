import type { AgentDashboard } from "./agent-dashboard.js";
import { addDecimals } from "./decimal.js";

/**
 * Equity for the dashboard's USDC/main-DEX scope, not non-USDC portfolio assets.
 * https://hyperliquid.gitbook.io/hyperliquid-docs/trading/account-abstraction-modes
 */
export function dashboardTotalEquity(snapshot: AgentDashboard): string | null {
  const live = snapshot.live;
  if (live && (live.status !== "connected" || !live.sections.mode.fresh || !live.sections.spot.fresh)) return null;
  switch (snapshot.accountAbstraction) {
    case "unifiedAccount":
      // Unified spot totals already include shared perp collateral and P&L.
      // Individual perp-DEX summaries are not independent account equity.
      return snapshot.spotUsdc;
    case "disabled":
    case "default":
      if (live && !live.sections.perps.fresh) return null;
      return snapshot.perpEquity === null || snapshot.spotUsdc === null
        ? null : addDecimals(snapshot.perpEquity, snapshot.spotUsdc);
    default:
      // Unknown modes and portfolio margin need a complete portfolio valuation.
      return null;
  }
}

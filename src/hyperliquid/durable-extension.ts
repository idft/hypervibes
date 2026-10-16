import { Type, type Static } from "@earendil-works/pi-ai";
import { createOrderTools, toolResult } from "./orders/order-tools.js";
import type { OrderService } from "./orders/order-service.js";
import type { HyperliquidService } from "./service.js";
import { defineExtension, defineTool, section } from "@earendil-works/pi-durable";

const nullable = Type.Union([Type.String(), Type.Null()]);
const freshness = Type.Object({ updatedAt: Type.Union([Type.Number(), Type.Null()]), fresh: Type.Boolean(), source: Type.Union([Type.Literal("live"), Type.Literal("http"), Type.Literal("unavailable")]) });
const AccountStatusSchema = Type.Object({
  address: nullable, scope: Type.Literal("main-dex-perpetuals-and-spot-usdc"), observedAt: Type.Number(),
  accountAbstraction: Type.Union([Type.Literal("unifiedAccount"), Type.Literal("portfolioMargin"), Type.Literal("disabled"), Type.Literal("default"), Type.Null()]),
  perpEquity: nullable, spotUsdc: nullable, spotUsdcHold: nullable,
  positions: Type.Union([Type.Array(Type.Object({ coin: Type.String(), size: Type.String(), entryPrice: Type.String(), unrealizedPnl: Type.String(), liquidationPrice: nullable, leverage: Type.Number() })), Type.Null()]),
  orders: Type.Union([Type.Array(Type.Object({ id: Type.Number(), coin: Type.String(), side: Type.String(), size: Type.String(), price: Type.String(), type: Type.String() })), Type.Null()]),
  sections: Type.Object({ perps: freshness, spot: freshness, orders: freshness, mode: freshness }), errors: Type.Array(Type.String()),
});
/** Trusted host binding: the model cannot choose an account or supply credentials. */
export function createHyperliquidExtension(service: Pick<HyperliquidService, "agentAccountStatus">, userId: string, agentId: string, orders: OrderService) {
  return defineExtension({
    name: "hyperliquid",
    sections: [section("hyperliquid", () => "Use get_account to read this Agent's assigned Hyperliquid account: main-DEX perpetual positions/orders, spot USDC total and holds, and accountAbstraction (queried for this exact account). Decimal strings preserve exchange precision. Check each section's freshness, timestamps, and errors; null/stale data is not zero or an empty account. In unifiedAccount mode, spot USDC is the single shared balance collateralizing USDC-settled cross-margin perps and used for spot trading; individual perp-DEX balance summaries are not meaningful account equity. In portfolioMargin mode, eligible spot assets and perps share portfolio collateral, but this tool only includes spot USDC, not the whole portfolio. Never conclude the account has zero collateral from perpEquity=0 in either mode. In standard/manual mode, spot and perp balances are separate; do not assume spot USDC automatically collateralizes perps. Do not infer the mode from the application's supported/default mode, or guess if mode lookup failed. Spot total is not available buying power: holds, existing positions, margin requirements, and exposure on other DEXs may constrain it. Do not add spotUsdc and perpEquity into a total in unified/portfolio mode or double-count shared collateral. Report spot USDC explicitly when summarizing the account. Other spot tokens, spot orders, and other DEX positions/orders are outside this view. Order tools place/cancel real orders autonomously on this Agent's assigned account only. Use exact decimal strings and stable unique request_id values. Size is base-asset quantity, not a USDC amount. Main-DEX perps require a minimum order notional of $10 (size × price), in addition to exchange size/tick precision. Check intended notional before submitting; never silently increase the user's intended exposure to satisfy the minimum. Exchange validation is authoritative, including any reduce-only/closing exceptions; do not assume an exception or retry a rejected order unchanged. Reuse the same ID and identical arguments after an interruption; never use a new ID to retry an unknown outcome. Query get_order_request/get_order to reconcile. Unknown/not-found is not rejection or cancellation. Inspect every leg: accepted is not filled; a rejected TP/SL can leave exposure unprotected. Market entries are 0.5%-guarded IOC and can partially fill. Native attached TP/SL activates only on full parent fill (or certain venue margin-cancel paths); manually canceling a partially filled parent also cancels its protection. Triggers use mark prices; market triggers use a 10% guard and may not fully fill. Standalone triggers are fixed-size reduce-only. Cancel-all can remove protective stops and does not close positions. No transfers, leverage changes, or trading on other accounts/DEXs are exposed. Reference: https://hyperliquid.gitbook.io/hyperliquid-docs/trading/account-abstraction-modes")],
    tools: [defineTool({
      name: "get_account",
      description: "Read this Agent's own Hyperliquid account mode, spot USDC total/holds, main-DEX perpetual positions/orders, and raw perp equity summary with per-section freshness. Unified/portfolio accounts use spot collateral; raw perp equity is not account equity. Reuses the live feed with HTTP repair.",
      parameters: Type.Object({}, { additionalProperties: false }),
      replay: "safe",
      structuredOutputSchema: AccountStatusSchema,
      execute: async (_args, _api, context) => {
        const status: Static<typeof AccountStatusSchema> = await service.agentAccountStatus(userId, agentId, context.abortSignal);
        return toolResult(status);
      },
    }), ...createOrderTools(orders, userId, agentId)],
  });
}

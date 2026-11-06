import { Type, type Static, type TSchema } from "typebox";

const nullable = <T extends TSchema>(schema: T) => Type.Union([schema, Type.Null()]);
const decimal = Type.String({ pattern: "^-?[0-9]+(\\.[0-9]+)?(?![\\s\\S])" });
const unsigned = Type.String({ pattern: "^[0-9]+(\\.[0-9]+)?(?![\\s\\S])" });
const integer = Type.Integer({ minimum: 0, maximum: Number.MAX_SAFE_INTEGER });
const symbol = Type.String({ minLength: 1 });
const summary = Type.Object({
  accountValue: decimal, totalNtlPos: unsigned, totalRawUsd: decimal, totalMarginUsed: unsigned,
});

/** SDK 0.33.3 response fields; financial values remain decimal strings. */
export const AccountPerpStateSchema = Type.Object({
  marginSummary: summary, crossMarginSummary: summary,
  crossMaintenanceMarginUsed: unsigned, withdrawable: unsigned, time: integer,
  assetPositions: Type.Array(Type.Object({
    type: Type.Literal("oneWay"),
    position: Type.Object({
      coin: symbol, szi: decimal,
      leverage: Type.Union([
        Type.Object({ type: Type.Literal("cross"), value: Type.Integer({ minimum: 1 }) }),
        Type.Object({ type: Type.Literal("isolated"), value: Type.Integer({ minimum: 1 }), rawUsd: decimal }),
      ]),
      entryPx: unsigned, positionValue: unsigned, unrealizedPnl: decimal,
      returnOnEquity: decimal, liquidationPx: nullable(unsigned), marginUsed: unsigned,
      maxLeverage: Type.Integer({ minimum: 1 }),
      cumFunding: Type.Object({ allTime: decimal, sinceOpen: decimal, sinceChange: decimal }),
    }),
  })),
});

const balance = { coin: symbol, total: decimal, hold: decimal, entryNtl: decimal };
const tokenRatio = Type.Array(Type.Tuple([integer, unsigned]));
export const AccountSpotStateSchema = Type.Object({
  balances: Type.Array(Type.Union([
    Type.Object({ ...balance, token: integer, spotHold: Type.Optional(decimal), ltv: Type.Optional(unsigned), borrowed: Type.Optional(unsigned), supplied: Type.Optional(unsigned) }),
    Type.Object({ ...balance, coin: Type.String({ pattern: "^(\\+[0-9]+|o[0-9]+)(?![\\s\\S])" }) }),
  ])),
  portfolioMarginEnabled: Type.Optional(Type.Boolean()),
  evmEscrows: Type.Optional(Type.Array(Type.Object({ coin: symbol, token: integer, total: unsigned }))),
  portfolioMarginRatio: Type.Optional(unsigned),
  tokenToPortfolioBorrowRatio: Type.Optional(tokenRatio),
  tokenToPortfolioSupplyRatio: Type.Optional(tokenRatio),
  tokenToAvailableAfterMaintenance: Type.Optional(tokenRatio),
});

// Children are flattened into separate records with parentOid. Missing optional
// SDK fields remain absent; they must not be synthesized as exchange evidence.
export const AccountOrderSchema = Type.Object({
  coin: symbol, side: Type.Union([Type.Literal("B"), Type.Literal("A")]),
  limitPx: unsigned, sz: unsigned, origSz: unsigned, oid: integer, timestamp: integer,
  triggerCondition: Type.String(), isTrigger: Type.Boolean(), triggerPx: unsigned,
  isPositionTpsl: Type.Boolean(), reduceOnly: Type.Boolean(), orderType: symbol,
  cloid: Type.Optional(nullable(Type.String({ pattern: "^0x[a-fA-F0-9]{32}(?![\\s\\S])" }))),
  tif: Type.Optional(nullable(Type.String())), parentOid: nullable(integer),
  orderIdKnown: Type.Boolean(), sourcePath: Type.Array(integer),
});
export const AccountModeSchema = Type.Union([
  Type.Literal("unifiedAccount"), Type.Literal("portfolioMargin"), Type.Literal("disabled"),
  Type.Literal("default"), Type.Literal("dexAbstraction"),
]);
const section = Type.Object({
  updatedAt: nullable(integer), fresh: Type.Boolean(),
  source: Type.Union([Type.Literal("http"), Type.Literal("unavailable")]),
});
const unknownAmount = Type.Object({ value: Type.Null(), reason: Type.String() });
const legacyPosition = Type.Object({
  coin: symbol, size: decimal, entryPrice: unsigned, unrealizedPnl: decimal,
  liquidationPrice: nullable(unsigned), leverage: Type.Number(),
});
const legacyOrder = Type.Object({
  id: integer, coin: symbol, side: Type.String(), size: unsigned, price: unsigned, type: Type.String(),
  triggerPrice: Type.Optional(unsigned), timestamp: Type.Optional(integer), positionTpsl: Type.Optional(Type.Literal(true)),
});

export const AccountStatusSchema = Type.Object({
  address: nullable(Type.String()), scope: Type.Literal("account-wide-perpetuals-and-spot"),
  startedAt: integer, observedAt: integer, accountAbstraction: nullable(AccountModeSchema),
  equity: unknownAmount, availableMargin: unknownAmount,
  coverage: Type.Object({
    complete: Type.Boolean({ description: "All requested HTTP source sections succeeded and remain fresh at observedAt. This is not complete opening-risk verification." }),
    openingRiskVerified: Type.Literal(false), perpDexs: nullable(Type.Array(Type.String())), discovery: section,
    limitations: Type.Array(Type.String()),
  }),
  perps: Type.Array(Type.Object({
    dex: Type.String(), state: nullable(AccountPerpStateSchema), orders: nullable(Type.Array(AccountOrderSchema)),
    sections: Type.Object({ state: section, orders: section }),
  })),
  spot: Type.Object({
    state: nullable(AccountSpotStateSchema), orders: nullable(Type.Array(AccountOrderSchema)),
    sections: Type.Object({ balances: section, orders: section }),
  }),
  outcomeOrders: Type.Object({ orders: nullable(Type.Array(AccountOrderSchema)), section }),
  // Compatibility fields cover the main perpetual DEX and spot USDC only.
  perpEquity: nullable(decimal), spotUsdc: nullable(decimal), spotUsdcHold: nullable(decimal),
  positions: nullable(Type.Array(legacyPosition)), orders: nullable(Type.Array(legacyOrder)),
  sections: Type.Object({ perps: section, spot: section, orders: section, mode: section }),
  errors: Type.Array(Type.String()),
});
export type AccountStatus = Static<typeof AccountStatusSchema>;

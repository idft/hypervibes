import { Type, type Static } from "@earendil-works/pi-ai";

const object = { additionalProperties: false } as const;
export const Decimal = Type.String({ pattern: "^(0|[1-9][0-9]*)(\\.[0-9]+)?$", maxLength: 80, description: "Exact plain decimal string, never a JSON number or exponent." });
export const RequestId = Type.String({ minLength: 1, maxLength: 120, pattern: "^[a-zA-Z0-9_-]+$", description: "Unique logical request ID. Reuse with identical arguments on retry; never issue a new ID to retry an unknown outcome." });
const SymbolName = Type.String({ minLength: 1, maxLength: 30, pattern: "^[A-Za-z0-9_-]+$", description: "Exact main-DEX perpetual symbol from exchange metadata (e.g. BTC). No spot or HIP-3 symbols." });
const Trigger = Type.Object({ trigger_price: Decimal, limit_price: Type.Optional(Decimal), size: Type.Optional(Decimal) }, object);
export const OrderInputSchema = Type.Object({
  symbol: SymbolName, side: Type.Union([Type.Literal("buy"), Type.Literal("sell")]),
  order_type: Type.Union([Type.Literal("limit"), Type.Literal("market"), Type.Literal("take_profit"), Type.Literal("stop_loss")]),
  size: Decimal, price: Type.Optional(Decimal), trigger_price: Type.Optional(Decimal),
  time_in_force: Type.Optional(Type.Union([Type.Literal("gtc"), Type.Literal("ioc"), Type.Literal("alo")])),
  reduce_only: Type.Optional(Type.Boolean()),
  take_profits: Type.Optional(Type.Array(Trigger, { maxItems: 1, description: "At most one attached native OCO take-profit. Use a standalone reduce-only trigger for position-level protection." })),
  stop_losses: Type.Optional(Type.Array(Trigger, { maxItems: 1 })),
}, object);
export const SubmitSchema = Type.Object({ request_id: RequestId, decision_ref: Type.Optional(Type.String({ pattern: "^m[1-9][0-9]*$", maxLength: 17 })), orders: Type.Array(OrderInputSchema, { minItems: 1, maxItems: 20 }) }, object);
export const CancelInputSchema = Type.Object({ symbol: SymbolName, oid: Type.String({ pattern: "^[1-9][0-9]*$", maxLength: 16, description: "Exchange order ID as a decimal string." }) }, object);
export const CancelSchema = Type.Object({ request_id: RequestId, orders: Type.Array(CancelInputSchema, { minItems: 1, maxItems: 100 }) }, object);
export const CancelAllSchema = Type.Object({ request_id: RequestId, symbol: Type.Optional(SymbolName) }, object);
export const ListSchema = Type.Object({
  source: Type.Optional(Type.Union([Type.Literal("open"), Type.Literal("local")])), symbol: Type.Optional(SymbolName), status: Type.Optional(Type.String({ maxLength: 60 })),
  limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 100 })), offset: Type.Optional(Type.Integer({ minimum: 0, maximum: 100000 })),
}, object);
export const GetSchema = Type.Object({ order_id: Type.String({ minLength: 1, maxLength: 80 }), include_events: Type.Optional(Type.Boolean()) }, object);
export type OrderInput = Static<typeof OrderInputSchema>;
export type SubmitInput = Static<typeof SubmitSchema>;
export type CancelInput = Static<typeof CancelSchema>;
export type CancelAllInput = Static<typeof CancelAllSchema>;
export type ListInput = Static<typeof ListSchema>;
const NullableString = Type.Union([Type.String(), Type.Null()]);
export const OrderViewSchema = Type.Object({
  id: Type.String(), requestId: NullableString, symbol: Type.String(), side: Type.String(), kind: Type.String(),
  size: NullableString, price: NullableString, triggerPrice: NullableString, reduceOnly: Type.Boolean(),
  cloid: NullableString, exchangeOid: NullableString,
  commandStatus: Type.Union([Type.Literal("not_sent"), Type.Literal("accepted"), Type.Literal("rejected"), Type.Literal("unknown"), Type.Literal("reconciled")]),
  orderStatus: Type.String(), filledSize: NullableString, averageFillPrice: NullableString,
  decisionRef: Type.Optional(NullableString), executionOrigin: Type.Optional(Type.String()),
  updatedAt: Type.Number(), venueTimestamp: Type.Union([Type.Number(), Type.Null()]), source: Type.String(), error: NullableString,
}, object);
export type OrderView = Static<typeof OrderViewSchema>;
export const OperationSchema = Type.Object({
  operationId: Type.String(), requestId: Type.String(), kind: Type.String(),
  results: Type.Array(OrderViewSchema), uncertain: Type.Boolean(), warnings: Type.Array(Type.String()),
  execution: Type.Optional(Type.Object({ origin: Type.String(), decisionRef: NullableString, coverage: Type.String() })),

}, object);
export type OperationResult = Static<typeof OperationSchema>;
export const EventSchema = Type.Object({ id: Type.Number(), source: Type.String(), time: Type.Number(), data: Type.String() }, object);
export const GetResultSchema = Type.Object({ order: Type.Union([OrderViewSchema, Type.Null()]), events: Type.Array(EventSchema), error: NullableString, observedAt: Type.Number() }, object);
export const ListResultSchema = Type.Object({ items: Type.Array(OrderViewSchema), source: Type.String(), observedAt: Type.Number(), nextOffset: Type.Union([Type.Number(), Type.Null()]), coverage: Type.String() }, object);
export type GetResult = Static<typeof GetResultSchema>;
export type ListResult = Static<typeof ListResultSchema>;

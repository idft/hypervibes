import { Type } from "@earendil-works/pi-ai";

const nonnegativeInteger = Type.Integer({ minimum: 0 });
const nullableInteger = Type.Union([nonnegativeInteger, Type.Null()]);
const epochSeconds = Type.Integer({ minimum: 0, description: "Unix epoch seconds (UTC)" });
const nullableEpochSeconds = Type.Union([epochSeconds, Type.Null()]);
const epochMilliseconds = Type.Integer({ minimum: 0, description: "Unix epoch milliseconds (UTC)" });
const nullableEpochMilliseconds = Type.Union([epochMilliseconds, Type.Null()]);
const nullableMoney = Type.Union([Type.String(), Type.Null()]);
const metadataFields = {
  id: Type.String(), agentId: Type.String(), title: Type.String(), description: Type.String(), mediaType: Type.String(), size: Type.Integer({ minimum: 1 }),
  tags: Type.Array(Type.String(), { maxItems: 12 }), instrument: Type.String(), interval: Type.String(), createdAt: epochMilliseconds, expiresAt: nullableEpochMilliseconds,
  conversationId: nullableInteger, taskId: nullableInteger, protectedByMemory: Type.Optional(Type.Boolean()),
};
export const ArtifactMetaSchema = Type.Object(metadataFields);
export const ArtifactReadSchema = Type.Object({ ...metadataFields,
  attachments: Type.Array(Type.Object({ id: Type.String(), title: Type.String(), mediaType: Type.String() })),
  content: Type.Union([Type.String(), Type.Null()]), nextOffset: nullableInteger,
});
export const ArtifactListSchema = Type.Object({ items: Type.Array(ArtifactMetaSchema, { maxItems: 50 }), nextOffset: nullableInteger });
export const ExportSchema = Type.Object({ artifact: ArtifactMetaSchema, reference: Type.String(), summary: Type.Record(Type.String(), Type.Unknown()) });
export const CandleCoverageSchema = Type.Object({ complete: Type.Boolean(), returned: nonnegativeInteger, expected: Type.Number({ minimum: 0 }), sourceLimit: Type.String(), closedOnly: Type.Literal(true) });
export const DatasetSchema = Type.Object({ count: nonnegativeInteger, firstTime: nullableEpochSeconds, lastTime: nullableEpochSeconds });
const candle = Type.Object({ time: Type.Number({ description: "Candle open time in Unix epoch seconds (UTC)" }), open: Type.Number(), high: Type.Number(), low: Type.Number(), close: Type.Number(), volume: Type.Number() });
export const CandleResultSchema = Type.Union([Type.Object({ coin: Type.String(), interval: Type.String(), start: epochMilliseconds, end: epochMilliseconds, pageEnd: epochMilliseconds, dataset: DatasetSchema, candles: Type.Array(candle, { maxItems: 2000 }), nextStart: nullableEpochMilliseconds, coverage: CandleCoverageSchema }), ExportSchema]);
const trade = Type.Object({
  id: Type.String(), symbol: Type.String(), direction: Type.String(), status: Type.String(), openedAt: epochMilliseconds, lastActivityAt: epochMilliseconds, closedAt: nullableEpochMilliseconds,
  remainingSize: Type.String(), entrySize: Type.String(), exitSize: Type.String(), entryNotional: Type.String(), exitNotional: Type.String(),
  grossPnl: Type.String(), fees: Type.String(), funding: nullableMoney, net: nullableMoney, feeCoverage: Type.Boolean(), coverageReason: Type.Union([Type.String(), Type.Null()]),
});
export const JournalResultSchema = Type.Union([Type.Object({
  trades: Type.Optional(Type.Array(trade, { maxItems: 1000 })), next: Type.Union([Type.Object({ time: epochMilliseconds, id: Type.String() }), Type.Null()]),
  performance: Type.Object({ scope: Type.String(), closed: nonnegativeInteger, knownNetCount: nonnegativeInteger, knownNetSum: Type.String(), knownNetWins: nonnegativeInteger, grossPnl: Type.String(), fees: Type.String() }),
  coverage: Type.Object({ complete: Type.Boolean(), historyLimited: Type.Boolean(), filter: Type.String(), truncated: Type.Boolean(),
    catchUp: Type.Optional(Type.Object({ attempted: Type.Boolean(), limitation: Type.Union([Type.String(), Type.Null()]) })),
    sync: Type.Array(Type.Object({ stream: Type.String(), cursor: epochMilliseconds, updated_at: nullableEpochMilliseconds, error: Type.Union([Type.String(), Type.Null()]) })),
  }),
}), ExportSchema]);

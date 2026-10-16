import { Type } from "@earendil-works/pi-ai";
import { defineTool } from "@earendil-works/pi-durable";
import type { JsonValue } from "@earendil-works/chord";
import { CancelAllSchema, CancelSchema, GetResultSchema, GetSchema, ListResultSchema, ListSchema, OperationSchema, RequestId, SubmitSchema, type CancelInput, type SubmitInput } from "./order-contracts.js";
import { assertStringAmounts } from "./order-prepare.js";
import type { OrderService } from "./order-service.js";

/** Both model and nested/code-mode callers get the same typed, bounded value. */
export function toolResult<T extends JsonValue>(value: T) {
  return { output: [{ type: "text" as const, text: JSON.stringify(value) }], structuredOutput: value };
}
export function createOrderTools(service: OrderService, userId: string, agentId: string) {
  return [
    defineTool({ name: "submit_orders", description: "Place real main-DEX perpetual orders autonomously on this Agent's assigned account. Exact decimal-string size/prices; size is base-asset quantity. Hyperliquid main-DEX perps have a $10 minimum order notional (size × price), plus symbol-specific size/tick precision. Do not silently increase size to meet the minimum or bypass reduce-only constraints; report exchange rejections/exceptions. Limit GTC/IOC/ALO, market entries (0.5% guarded IOC), attached native TP/SL (one each), and standalone reduce-only TP/SL. Attached SL is market-on-trigger; triggers without a limit use a 10% guard. Persisted request_id deduplicates retries; never use a new ID to retry unknown outcomes. Inspect every result/leg; accepted does not mean filled or protected.",
      parameters: SubmitSchema, structuredOutputSchema: OperationSchema, replay: "safe", executionMode: "sequential",
      prepareArguments(args: unknown) { assertStringAmounts(args); return args as SubmitInput; },
      execute: async (args, _api, context) => toolResult(await service.submit(userId, agentId, args, context.abortSignal)),
    }),
    defineTool({ name: "cancel_orders", description: "Cancel explicit exchange OIDs on this Agent's own main-DEX perpetual account. Each item requires symbol and oid (decimal string). May remove protective TP/SL; does not close positions. Use the same request_id for retries; inspect per-item results and unknown outcomes.",
      parameters: CancelSchema, structuredOutputSchema: OperationSchema, replay: "safe", executionMode: "sequential",
      prepareArguments(args: unknown) { assertStringAmounts(args); return args as CancelInput; },
      execute: async (args, _api, context) => toolResult(await service.cancel(userId, agentId, args, context.abortSignal)),
    }),
    defineTool({ name: "cancel_all_orders", description: "Cancel one captured set of current main-DEX perpetual orders, optionally for one symbol. Includes protective stops/TP; does not close positions. Replaying the same request_id never captures newly created orders. Maximum 100 targets; use explicit batches beyond that.",
      parameters: CancelAllSchema, structuredOutputSchema: OperationSchema, replay: "safe", executionMode: "sequential",
      execute: async (args, _api, context) => toolResult(await service.cancelAll(userId, agentId, args, context.abortSignal)),
    }),
    defineTool({ name: "list_orders", description: "List this Agent's main-DEX perpetual orders. source=open (default) reads live exchange orders, including external orders. source=local reads the latest 500 recorded submission requests, not full exchange history; states can lag. Optional symbol/status and bounded limit/offset. Use get_order for fresh status of a selected local order.",
      parameters: ListSchema, structuredOutputSchema: ListResultSchema, replay: "safe",
      execute: async (args, _api, context) => toolResult(await service.list(userId, agentId, args, context.abortSignal)),
    }),
    defineTool({ name: "get_order", description: "Query this Agent's main-DEX perpetual order by local ID, decimal-string exchange OID, or 128-bit hex CLOID. Reconciles local requests before returning. Optional include_events returns up to 100 latest local events, newest first. Not-found does not prove rejection, fill, or cancellation.",
      parameters: GetSchema, structuredOutputSchema: GetResultSchema, replay: "safe",
      execute: async (args, _api, context) => toolResult(await service.get(userId, agentId, args.order_id, args.include_events, context.abortSignal)),
    }),
    defineTool({ name: "get_order_request", description: "Read and reconcile an existing submit/cancel request_id, including all per-leg outcomes and uncertainty. Never sends/retries exchange actions. Use after timeouts/restarts instead of issuing a new request_id.",
      parameters: Type.Object({ request_id: RequestId }, { additionalProperties: false }), structuredOutputSchema: OperationSchema, replay: "safe",
      execute: async (args, _api, context) => toolResult(await service.request(userId, agentId, args.request_id, context.abortSignal)),
    }),
  ];
}

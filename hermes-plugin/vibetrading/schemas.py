FETCH_CANDLES_SCHEMA = {
    "name": "fetch_candles",
    "description": (
        "Retrieve OHLCV candlestick data directly from Hyperliquid using the "
        "official Hyperliquid Python SDK. Returns a JSON array of candles in "
        "the SDK-native shape [[timestamp, open, high, low, close, volume], ...]."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "instrument": {
                "type": "string",
                "description": (
                    "Instrument identifier. v1 may hardcode BTC/USD until the "
                    "instrument model is finalized."
                ),
            },
            "timeframe": {
                "type": "string",
                "description": (
                    "Candle interval, e.g. '1m', '5m', '15m', '1h', '4h', '1d'."
                ),
            },
            "limit": {
                "type": "integer",
                "description": "Maximum number of candles to return.",
                "minimum": 1,
            },
            "start_time": {
                "type": "integer",
                "description": "Optional start timestamp in milliseconds.",
            },
            "end_time": {
                "type": "integer",
                "description": "Optional end timestamp in milliseconds.",
            },
        },
        "required": ["instrument", "timeframe"],
    },
}

ANALYZE_MARKET_SCHEMA = {
    "name": "analyze_market",
    "description": (
        "Run local Python analysis on candle data returned by fetch_candles. "
        "Computes the requested indicators (SMA, EMA, RSI, MACD, support/resistance, "
        "etc.) and returns a JSON summary plus a short narrative."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "candles": {
                "type": "array",
                "description": "Candle payload from fetch_candles.",
                "items": {"type": "array"},
            },
            "indicators": {
                "type": "array",
                "description": "Optional list of indicator names to compute.",
                "items": {"type": "string"},
            },
        },
        "required": ["candles"],
    },
}

READ_MEMORY_SCHEMA = {
    "name": "read_memory",
    "description": (
        "Read structured memory records from the Vibetrading backend. "
        "Stub: backend memory subsystem is not implemented yet."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "symbol": {"type": "string"},
            "timeframe": {"type": "string"},
            "memory_type": {
                "type": "string",
                "enum": [
                    "observation",
                    "hypothesis",
                    "plan",
                    "outcome",
                    "reflection",
                ],
            },
            "active_only": {"type": "boolean", "default": False},
        },
    },
}

WRITE_MEMORY_SCHEMA = {
    "name": "write_memory",
    "description": (
        "Write structured memory records to the Vibetrading backend. "
        "Stub: backend memory subsystem is not implemented yet."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "memory_type": {
                "type": "string",
                "enum": [
                    "observation",
                    "hypothesis",
                    "plan",
                    "outcome",
                    "reflection",
                ],
            },
            "symbol": {"type": "string"},
            "timeframe": {"type": "string"},
            "summary": {"type": "string"},
            "thesis": {"type": "string"},
            "confidence": {"type": "number", "minimum": 0, "maximum": 1},
            "parent_memory_ids": {
                "type": "array",
                "items": {"type": "string"},
            },
        },
        "required": ["memory_type"],
    },
}

PLACE_ORDER_SCHEMA = {
    "name": "place_order",
    "description": (
        "Submit an order through the Vibetrading execution gateway "
        "(POST /api/v1/orders). Records every leg in hyperliquid.orders "
        "before forwarding to Hyperliquid. Linking to source memory is "
        "supported via memory_record_ids."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "symbol": {"type": "string", "description": "e.g. 'BTC'"},
            "side": {"type": "string", "enum": ["buy", "sell"]},
            "order_type": {"type": "string", "enum": ["limit", "market"]},
            "size": {
                "type": "string",
                "description": "Decimal string, e.g. '0.1'",
            },
            "price": {
                "type": "string",
                "description": "Required for limit, ignored for market.",
            },
            "time_in_force": {
                "type": "string",
                "enum": ["gtc", "ioc", "alo"],
                "default": "gtc",
            },
            "reduce_only": {"type": "boolean", "default": False},
            "take_profits": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "trigger_price": {"type": "string"},
                        "limit_price": {"type": "string"},
                        "size": {"type": "string"},
                    },
                    "required": ["trigger_price"],
                },
            },
            "stop_losses": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "trigger_price": {"type": "string"},
                        "limit_price": {"type": "string"},
                        "size": {"type": "string"},
                    },
                    "required": ["trigger_price"],
                },
            },
            "memory_record_ids": {
                "type": "array",
                "items": {"type": "string"},
            },
        },
        "required": ["symbol", "side", "order_type", "size"],
    },
}

CANCEL_ORDER_SCHEMA = {
    "name": "cancel_order",
    "description": (
        "Cancel one or more orders by exchange oid. "
        "POST /api/v1/orders/cancel. Returns per-input outcomes."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "orders": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "symbol": {"type": "string"},
                        "oid": {"type": "integer"},
                    },
                    "required": ["symbol", "oid"],
                },
            },
        },
        "required": ["orders"],
    },
}

CANCEL_ALL_SCHEMA = {
    "name": "cancel_all",
    "description": (
        "Cancel every resting order for the calling agent. "
        "POST /api/v1/orders/cancel-all?symbol=BTC."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "symbol": {
                "type": "string",
                "description": "Optional symbol filter.",
            },
        },
    },
}

LIST_ORDERS_SCHEMA = {
    "name": "list_orders",
    "description": (
        "List the agent's orders, newest first. Scoped by agent_key "
        "server-side. Use this to find exchange_oid for cancel_order."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "status": {
                "type": "string",
                "description": (
                    "open (alias for submitted|resting|partially_filled), "
                    "pending, or any exact status name."
                ),
            },
            "symbol": {
                "type": "string",
                "description": "Optional symbol filter.",
            },
        },
    },
}

GET_ORDER_SCHEMA = {
    "name": "get_order",
    "description": (
        "Fetch a single order by id. ?include=events appends the "
        "lifecycle events from hyperliquid.order_events."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "id": {"type": "string", "description": "Order id (uuid)."},
            "include": {
                "type": "string",
                "enum": ["events"],
            },
        },
        "required": ["id"],
    },
}

GET_ACCOUNT_STATUS_SCHEMA = {
    "name": "get_account_status",
    "description": (
        "Fetch current Hyperliquid account status directly from Hyperliquid "
        "using the agent's HYPERLIQUID_ADDRESS. Returns available balance, "
        "open positions, margin, recent fills, etc."
    ),
    "parameters": {
        "type": "object",
        "properties": {},
    },
}

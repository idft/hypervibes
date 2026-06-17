from .schemas import (
    ANALYZE_MARKET_SCHEMA,
    CANCEL_ALL_SCHEMA,
    CANCEL_ORDER_SCHEMA,
    FETCH_CANDLES_SCHEMA,
    GET_ACCOUNT_STATUS_SCHEMA,
    GET_ORDER_SCHEMA,
    LIST_ORDERS_SCHEMA,
    PLACE_ORDER_SCHEMA,
    READ_MEMORY_SCHEMA,
    WRITE_MEMORY_SCHEMA,
)


def _not_implemented(*args, **kwargs):
    return {"status": "not implemented"}


fetch_candles = _not_implemented
analyze_market = _not_implemented
read_memory = _not_implemented
write_memory = _not_implemented
place_order = _not_implemented
cancel_order = _not_implemented
cancel_all = _not_implemented
list_orders = _not_implemented
get_order = _not_implemented
get_account_status = _not_implemented


__all__ = [
    "ANALYZE_MARKET_SCHEMA",
    "CANCEL_ALL_SCHEMA",
    "CANCEL_ORDER_SCHEMA",
    "FETCH_CANDLES_SCHEMA",
    "GET_ACCOUNT_STATUS_SCHEMA",
    "GET_ORDER_SCHEMA",
    "LIST_ORDERS_SCHEMA",
    "PLACE_ORDER_SCHEMA",
    "READ_MEMORY_SCHEMA",
    "WRITE_MEMORY_SCHEMA",
    "analyze_market",
    "cancel_all",
    "cancel_order",
    "fetch_candles",
    "get_account_status",
    "get_order",
    "list_orders",
    "place_order",
    "read_memory",
    "write_memory",
]

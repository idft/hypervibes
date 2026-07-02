"""Vibetrading MCP server.

Runtime infrastructure that exposes Vibetrading backend endpoints to
OpenCode agents as MCP tools. See ``README.md`` for context.

The server:

* Loads ``.env`` from the current working directory (the agent workspace)
  defensively. Required vars: ``VIBETRADING_API_BASE_URL``,
  ``VIBETRADING_API_KEY``, ``VIBETRADING_AGENT_KEY``.
* Never prints secret values.
* Returns JSON-compatible dict/list values only.
* Maps backend HTTP errors to MCP errors without leaking the bearer token.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path
from typing import Any

import httpx
from dotenv import load_dotenv
from mcp.server.fastmcp import FastMCP


HTTP_TIMEOUT_SECONDS = 30.0

mcp = FastMCP("vibetrading")


def _load_config() -> tuple[str, str, str]:
    """Read required env vars, failing fast with a clear message.

    Returns ``(base_url, api_key, agent_key)``. ``base_url`` has any
    trailing slash stripped.
    """
    load_dotenv(dotenv_path=Path.cwd() / ".env")
    base_url = os.getenv("VIBETRADING_API_BASE_URL", "").strip().rstrip("/")
    api_key = os.getenv("VIBETRADING_API_KEY", "").strip()
    agent_key = os.getenv("VIBETRADING_AGENT_KEY", "").strip()
    missing = [
        name
        for name, value in (
            ("VIBETRADING_API_BASE_URL", base_url),
            ("VIBETRADING_API_KEY", api_key),
            ("VIBETRADING_AGENT_KEY", agent_key),
        )
        if not value
    ]
    if missing:
        joined = ", ".join(missing)
        raise RuntimeError(
            "Vibetrading MCP server is missing required environment "
            f"variables: {joined}"
        )
    return base_url, api_key, agent_key


CONFIG: tuple[str, str, str] = _load_config()


def _require_config() -> tuple[str, str, str]:
    """Return the cached config, reloading on first call after failure."""
    global CONFIG
    base_url, api_key, agent_key = CONFIG
    if not base_url or not api_key or not agent_key:
        CONFIG = _load_config()
        base_url, api_key, agent_key = CONFIG
    return CONFIG


def _headers() -> dict[str, str]:
    _, api_key, _ = _require_config()
    return {"Authorization": f"Bearer {api_key}"}


def _request(
    method: str,
    path: str,
    *,
    params: dict[str, Any] | None = None,
    json_body: Any = None,
) -> Any:
    """Make an authenticated request to the Vibetrading backend.

    Translates HTTP errors into ``RuntimeError`` with a redacted message.
    """
    base_url, _, _ = _require_config()
    url = f"{base_url}{path}"
    try:
        response = httpx.request(
            method,
            url,
            params=params,
            json=json_body,
            headers=_headers(),
            timeout=HTTP_TIMEOUT_SECONDS,
        )
    except httpx.HTTPError as exc:
        raise RuntimeError(f"Vibetrading request failed: {exc}") from exc

    if response.status_code >= 400:
        detail = response.text.strip()
        # The backend may echo the bearer token in some misconfigurations;
        # never let a raw response body leak credentials back to the LLM.
        detail = detail.replace(_require_config()[1], "[redacted]")
        raise RuntimeError(
            f"Vibetrading {method} {path} returned "
            f"{response.status_code}: {detail[:500]}"
        )

    if not response.content:
        return None
    try:
        return response.json()
    except ValueError as exc:
        raise RuntimeError(
            f"Vibetrading {method} {path} returned non-JSON body"
        ) from exc


def _require_nonblank(name: str, value: str) -> str:
    if not value or not value.strip():
        raise ValueError(f"{name} must not be blank")
    return value


def _require_limit(limit: int | None) -> int | None:
    if limit is None:
        return None
    if limit < 1:
        raise ValueError("limit must be >= 1")
    return limit


@mcp.tool()
def get_account() -> dict[str, Any]:
    """Return this agent's current Hyperliquid account snapshot."""
    result = _request("GET", "/api/v1/account")
    if not isinstance(result, dict):
        raise RuntimeError("Vibetrading /account returned unexpected shape")
    return result


@mcp.tool()
def get_latest_analysis(symbol: str, limit: int | None = None) -> list[dict[str, Any]]:
    """Return the latest analysis memory rows for ``symbol``.

    ``limit`` is optional; when provided it must be ``>= 1``. The
    ``memory_type`` filter is fixed to ``"analysis"`` on the server.
    """
    symbol = _require_nonblank("symbol", symbol)
    _require_limit(limit)
    params: dict[str, Any] = {"symbol": symbol, "memory_type": "analysis"}
    if limit is not None:
        params["limit"] = limit
    result = _request("GET", "/api/v1/memories/latest", params=params)
    if not isinstance(result, list):
        raise RuntimeError(
            "Vibetrading /memories/latest returned unexpected shape"
        )
    return result


@mcp.tool()
def get_market_analysis(symbol: str) -> dict[str, Any] | None:
    """Return the latest fresh market-analysis memory row for ``symbol``."""
    symbol = _require_nonblank("symbol", symbol)
    result = _request(
        "GET",
        "/api/v1/memories",
        params={"symbol": symbol, "memory_type": "market_analysis", "limit": 1},
    )
    if not isinstance(result, list):
        raise RuntimeError("Vibetrading /memories returned unexpected shape")
    if not result:
        return None
    first = result[0]
    if not isinstance(first, dict):
        raise RuntimeError("Vibetrading /memories returned unexpected shape")
    return first


@mcp.tool()
def list_memories(
    symbol: str | None = None,
    timeframe: str | None = None,
    memory_type: str | None = None,
    limit: int | None = None,
    include_expired: bool = False,
) -> list[dict[str, Any]]:
    """List memory rows visible to this agent.

    All filters are optional. ``include_expired`` defaults to ``False`` to
    match the backend's default staleness hiding.
    """
    params: dict[str, Any] = {}
    if symbol is not None:
        params["symbol"] = _require_nonblank("symbol", symbol)
    if timeframe is not None:
        params["timeframe"] = _require_nonblank("timeframe", timeframe)
    if memory_type is not None:
        params["memory_type"] = _require_nonblank("memory_type", memory_type)
    if limit is not None:
        params["limit"] = _require_limit(limit)
    if include_expired:
        params["include_expired"] = "true"
    result = _request("GET", "/api/v1/memories", params=params or None)
    if not isinstance(result, list):
        raise RuntimeError("Vibetrading /memories returned unexpected shape")
    return result


@mcp.tool()
def list_orders(
    status: str | None = None,
    symbol: str | None = None,
) -> list[dict[str, Any]]:
    """List orders visible to this agent.

    Both filters are optional. When provided, ``status`` and ``symbol`` are
    forwarded to the backend unchanged.
    """
    params: dict[str, Any] = {}
    if status is not None:
        params["status"] = _require_nonblank("status", status)
    if symbol is not None:
        params["symbol"] = _require_nonblank("symbol", symbol)
    result = _request("GET", "/api/v1/orders", params=params or None)
    if not isinstance(result, list):
        raise RuntimeError("Vibetrading /orders returned unexpected shape")
    return result


@mcp.tool()
def get_order(order_id: str, include_events: bool = False) -> dict[str, Any]:
    """Fetch a single order by id.

    Set ``include_events=True`` to also return the order's event log.
    """
    order_id = _require_nonblank("order_id", order_id)
    params: dict[str, Any] | None = (
        {"include": "events"} if include_events else None
    )
    result = _request("GET", f"/api/v1/orders/{order_id}", params=params)
    if not isinstance(result, dict):
        raise RuntimeError("Vibetrading /orders/{id} returned unexpected shape")
    return result


@mcp.tool()
def write_memory(
    symbol: str,
    memory_type: str,
    summary: str,
    content: str,
    timeframe: str | None = None,
    metadata: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Persist a memory for this agent.

    Required: ``symbol``, ``memory_type``, ``summary``, ``content``.
    ``timeframe`` is optional. For a general memory, omit the ``timeframe``
    argument entirely; do not pass an empty string. ``metadata`` must be a
    JSON object when provided; ``None`` is stored as an empty object.
    """
    symbol = _require_nonblank("symbol", symbol)
    memory_type = _require_nonblank("memory_type", memory_type)
    summary = _require_nonblank("summary", summary)
    content = _require_nonblank("content", content)
    if timeframe is not None:
        timeframe = _require_nonblank("timeframe", timeframe)
    if metadata is not None and not isinstance(metadata, dict):
        raise ValueError("metadata must be a JSON object")
    body: dict[str, Any] = {
        "symbol": symbol,
        "memory_type": memory_type,
        "summary": summary,
        "content": content,
    }
    if timeframe is not None:
        body["timeframe"] = timeframe
    body["metadata"] = metadata if metadata is not None else {}
    result = _request("POST", "/api/v1/memories", json_body=body)
    if not isinstance(result, dict):
        raise RuntimeError("Vibetrading /memories POST returned unexpected shape")
    return result


@mcp.tool()
def submit_orders(orders: list[dict[str, Any]]) -> dict[str, Any]:
    """Submit one or more orders through the Vibetrading backend.

    ``orders`` is the same list shape the backend expects on
    ``POST /api/v1/orders``. This is a real backend action: the server
    selects instruments, signs the request, and submits to Hyperliquid.
    The MCP server does not hold or use any private key.
    """
    if not isinstance(orders, list) or not orders:
        raise ValueError("orders must be a non-empty list")
    for index, order in enumerate(orders):
        if not isinstance(order, dict):
            raise ValueError(f"orders[{index}] must be a JSON object")
    result = _request("POST", "/api/v1/orders", json_body={"orders": orders})
    if not isinstance(result, dict):
        raise RuntimeError("Vibetrading /orders POST returned unexpected shape")
    return result


@mcp.tool()
def cancel_orders(orders: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Cancel one or more orders through the Vibetrading backend.

    ``orders`` is the same list shape the backend expects on
    ``POST /api/v1/orders/cancel``. Returns one outcome per requested
    cancel.
    """
    if not isinstance(orders, list) or not orders:
        raise ValueError("orders must be a non-empty list")
    for index, order in enumerate(orders):
        if not isinstance(order, dict):
            raise ValueError(f"orders[{index}] must be a JSON object")
    result = _request("POST", "/api/v1/orders/cancel", json_body={"orders": orders})
    if not isinstance(result, list):
        raise RuntimeError(
            "Vibetrading /orders/cancel returned unexpected shape"
        )
    return result


@mcp.tool()
def cancel_all_orders(symbol: str | None = None) -> dict[str, Any]:
    """Cancel all open orders, optionally restricted to a single symbol.

    Real backend action. The MCP server does not hold or use any
    private key.
    """
    params: dict[str, Any] | None = None
    if symbol is not None:
        symbol = _require_nonblank("symbol", symbol)
        params = {"symbol": symbol}
    result = _request("POST", "/api/v1/orders/cancel-all", params=params)
    if not isinstance(result, dict):
        raise RuntimeError(
            "Vibetrading /orders/cancel-all returned unexpected shape"
        )
    return result


def main() -> None:
    # Fail fast on missing config so OpenCode gets a clear stderr message
    # instead of a half-initialised server.
    _require_config()
    mcp.run()


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:  # noqa: BLE001
        print(f"vibetrading MCP server failed to start: {exc}", file=sys.stderr)
        raise

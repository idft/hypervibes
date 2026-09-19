"""HyperVibes MCP server.

Runtime infrastructure that exposes HyperVibes backend endpoints to
OpenCode agents as MCP tools. See ``README.md`` for context.

The server:

* Loads ``.env`` from the current working directory (the agent workspace)
  defensively. Required vars: ``HYPERVIBES_API_BASE_URL``,
  ``HYPERVIBES_API_KEY``, ``HYPERVIBES_AGENT_KEY``.
* Never prints secret values.
* Returns JSON-compatible dict/list values only.
* Maps backend HTTP errors to MCP errors without leaking the bearer token.
"""

from __future__ import annotations

import logging
import os
import sys
from decimal import Decimal
from pathlib import Path
from typing import Annotated, Any, Literal

import httpx
from dotenv import load_dotenv
from mcp.server.fastmcp import FastMCP
from pydantic import BaseModel, ConfigDict, Field, StrictInt, field_validator, model_validator


HTTP_TIMEOUT_SECONDS = 30.0

mcp = FastMCP("hypervibes")


PositiveDecimal = Annotated[Decimal, Field(gt=0)]
NonBlankString = Annotated[str, Field(min_length=1)]


class _StrictToolInput(BaseModel):
    """Base model that keeps agent tool payloads aligned with the API contract."""

    model_config = ConfigDict(extra="forbid")


class TriggerOrderInput(_StrictToolInput):
    """One take-profit or stop-loss leg attached to an entry order."""

    trigger_price: PositiveDecimal
    limit_price: PositiveDecimal | None = None
    size: PositiveDecimal | None = None


class SubmitOrderInput(_StrictToolInput):
    """A single opening, closing, or position-management order."""

    symbol: NonBlankString = Field(description="Canonical instrument symbol, for example ETH.")
    side: Literal["buy", "sell"]
    order_type: Literal["limit", "market"]
    size: PositiveDecimal = Field(description="Base-asset order quantity.")
    price: PositiveDecimal | None = Field(
        default=None,
        description="Required positive limit price when order_type is limit.",
    )
    time_in_force: Literal["gtc", "ioc", "alo"] | None = Field(
        default=None,
        description="Optional limit-order time in force. Omit for gtc.",
    )
    reduce_only: bool = False
    take_profits: list[TriggerOrderInput] = Field(default_factory=list)
    stop_losses: list[TriggerOrderInput] = Field(default_factory=list)
    memory_record_ids: list[NonBlankString] = Field(default_factory=list)
    attribution_source: Literal["agent", "manual"] = "agent"

    @field_validator("symbol")
    @classmethod
    def symbol_must_not_be_blank(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("symbol must not be blank")
        return value

    @model_validator(mode="after")
    def limit_orders_require_price(self) -> SubmitOrderInput:
        if self.order_type == "limit" and self.price is None:
            raise ValueError("price is required for limit orders")
        return self


class CancelOrderInput(_StrictToolInput):
    """An exchange order cancellation request."""

    symbol: NonBlankString = Field(description="Canonical instrument symbol, for example ETH.")
    oid: Annotated[
        StrictInt,
        Field(
            ge=1,
            le=2**64 - 1,
            description="Numeric exchange-assigned order ID, not a quoted string.",
        ),
    ]

    @field_validator("symbol")
    @classmethod
    def symbol_must_not_be_blank(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("symbol must not be blank")
        return value


# Resolve postponed aliases before FastMCP inspects these models for tool schemas.
SubmitOrderInput.model_rebuild()
CancelOrderInput.model_rebuild()


# MCP uses stdout for its JSON-RPC transport. Keep operational diagnostics on
# stderr so they cannot corrupt tool responses.
LOGGER = logging.getLogger("hypervibes.mcp")
LOGGER.setLevel(logging.INFO)
if not LOGGER.handlers:
    formatter = logging.Formatter("%(asctime)s %(levelname)s %(message)s")
    stderr_handler = logging.StreamHandler(sys.stderr)
    stderr_handler.setFormatter(formatter)
    LOGGER.addHandler(stderr_handler)
LOGGER.propagate = False


def _redirect_stderr_to_container_log() -> None:
    """Bypass OpenCode's MCP stderr capture without touching stdio stdout."""
    try:
        container_stdout = os.open("/proc/1/fd/1", os.O_WRONLY)
        try:
            os.dup2(container_stdout, sys.stderr.fileno())
        finally:
            os.close(container_stdout)
    except OSError:
        # This path is unavailable outside the container, such as unit tests.
        pass


def _is_broken_stdio_shutdown(error: BaseException) -> bool:
    """Recognize FastMCP's expected error when OpenCode closes stdio."""
    if isinstance(error, BaseExceptionGroup):
        return bool(error.exceptions) and all(
            _is_broken_stdio_shutdown(exception) for exception in error.exceptions
        )
    return error.__class__.__name__ == "BrokenResourceError"


def _load_config() -> tuple[str, str, str]:
    """Read required env vars, failing fast with a clear message.

    Returns ``(base_url, api_key, agent_key)``. ``base_url`` has any
    trailing slash stripped.
    """
    load_dotenv(dotenv_path=Path.cwd() / ".env")
    base_url = os.getenv("HYPERVIBES_API_BASE_URL", "").strip().rstrip("/")
    api_key = os.getenv("HYPERVIBES_API_KEY", "").strip()
    agent_key = os.getenv("HYPERVIBES_AGENT_KEY", "").strip()
    missing = [
        name
        for name, value in (
            ("HYPERVIBES_API_BASE_URL", base_url),
            ("HYPERVIBES_API_KEY", api_key),
            ("HYPERVIBES_AGENT_KEY", agent_key),
        )
        if not value
    ]
    if missing:
        joined = ", ".join(missing)
        raise RuntimeError(
            "HyperVibes MCP server is missing required environment "
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
    headers = {"Authorization": f"Bearer {api_key}"}
    conversation_id = os.getenv("HYPERVIBES_CONVERSATION_ID", "").strip()
    if conversation_id:
        headers["X-HyperVibes-Conversation-Id"] = conversation_id
    return headers


def _request(
    method: str,
    path: str,
    *,
    params: dict[str, Any] | None = None,
    json_body: Any = None,
) -> Any:
    """Make an authenticated request to the HyperVibes backend.

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
        raise RuntimeError(f"HyperVibes request failed: {exc}") from exc

    if response.status_code >= 400:
        detail = response.text.strip()
        # Do not log the raw body: a misconfigured backend could echo
        # credentials or memory content in it. Preserve the existing redacted
        # error for the tool caller, which is useful for actionable API errors.
        detail = detail.replace(_require_config()[1], "[redacted]")
        LOGGER.warning(
            "hypervibes_mcp_request_failed method=%s path=%s params=%r status=%s",
            method,
            path,
            params or {},
            response.status_code,
        )
        raise RuntimeError(
            f"HyperVibes {method} {path} returned "
            f"{response.status_code}: {detail[:500]}"
        )

    if not response.content:
        LOGGER.info(
            "hypervibes_mcp_request method=%s path=%s params=%r status=%s response=empty",
            method,
            path,
            params or {},
            response.status_code,
        )
        return None
    try:
        result = response.json()
    except ValueError as exc:
        raise RuntimeError(
            f"HyperVibes {method} {path} returned non-JSON body"
        ) from exc
    if isinstance(result, list):
        response_shape = f"list:{len(result)}"
    elif isinstance(result, dict):
        response_shape = "object"
    else:
        response_shape = type(result).__name__
    LOGGER.info(
        "hypervibes_mcp_request method=%s path=%s params=%r status=%s response=%s",
        method,
        path,
        params or {},
        response.status_code,
        response_shape,
    )
    return result


def _require_nonblank(name: str, value: str) -> str:
    if not value or not value.strip():
        raise ValueError(f"{name} must not be blank")
    return value


def _require_sub_agent_id(sub_agent_id: int) -> int:
    if not isinstance(sub_agent_id, int) or isinstance(sub_agent_id, bool) or sub_agent_id <= 0:
        raise ValueError("sub_agent_id must be a positive integer")
    return sub_agent_id


def _require_strategy_prompt_response(value: Any) -> dict[str, Any]:
    if (
        not isinstance(value, dict)
        or set(value)
        != {"revision_id", "target_sub_agent_id", "target_sub_agent_key", "prompt", "updated_at"}
        or not isinstance(value["revision_id"], int)
        or not isinstance(value["target_sub_agent_id"], int)
        or not isinstance(value["target_sub_agent_key"], str)
        or not isinstance(value["prompt"], str)
        or not isinstance(value["updated_at"], str)
    ):
        raise RuntimeError("HyperVibes strategy prompt returned unexpected shape")
    return value


def _require_limit(limit: int | None) -> int | None:
    if limit is None:
        return None
    if limit < 1:
        raise ValueError("limit must be >= 1")
    return limit


def _require_offset(offset: int | None) -> int | None:
    if offset is None:
        return None
    if offset < 0:
        raise ValueError("offset must be >= 0")
    return offset


def _require_indicator_id(indicator_id: str) -> str:
    return _require_nonblank("indicator_id", indicator_id)


def _require_indicator_fields(
    name: str,
    timeframe: str,
    instrument_ids: list[str],
    source: str,
    input_values: dict[str, Any] | None,
) -> dict[str, Any]:
    _require_nonblank("name", name)
    _require_nonblank("timeframe", timeframe)
    _require_nonblank("source", source)
    if not isinstance(instrument_ids, list) or not instrument_ids or not all(
        isinstance(instrument_id, str) and instrument_id.strip()
        for instrument_id in instrument_ids
    ):
        raise ValueError("instrument_ids must be a non-empty list of nonblank IDs")
    if len(set(instrument_ids)) != len(instrument_ids):
        raise ValueError("instrument_ids must not contain duplicates")
    if input_values is not None and not isinstance(input_values, dict):
        raise ValueError("input_values must be a JSON object")
    return {
        "name": name,
        "timeframe": timeframe,
        "instrument_ids": instrument_ids,
        "source": source,
        "input_values": input_values if input_values is not None else {},
    }


@mcp.tool()
def get_account() -> dict[str, Any]:
    """Return this agent's current Hyperliquid account snapshot."""
    result = _request("GET", "/api/v1/account")
    if not isinstance(result, dict):
        raise RuntimeError("HyperVibes /account returned unexpected shape")
    return result


@mcp.tool()
def list_strategy_prompts() -> list[dict[str, Any]]:
    """List this agent's editable strategy prompts for chat review."""
    result = _request("GET", "/api/v1/strategy-prompts")
    if not isinstance(result, list):
        raise RuntimeError("HyperVibes strategy prompts returned unexpected shape")
    return [_require_strategy_prompt_response(prompt) for prompt in result]


@mcp.tool()
def get_strategy_prompt(sub_agent_id: int) -> dict[str, Any]:
    """Get one editable strategy prompt for chat review."""
    sub_agent_id = _require_sub_agent_id(sub_agent_id)
    result = _request("GET", f"/api/v1/strategy-prompts/{sub_agent_id}")
    return _require_strategy_prompt_response(result)


@mcp.tool()
def update_strategy_prompt(sub_agent_id: int, prompt: str) -> dict[str, Any]:
    """Update one strategy prompt from chat; blank prompts are allowed."""
    sub_agent_id = _require_sub_agent_id(sub_agent_id)
    if not isinstance(prompt, str):
        raise ValueError("prompt must be a string")
    result = _request(
        "PUT",
        f"/api/v1/strategy-prompts/{sub_agent_id}",
        json_body={"prompt": prompt},
    )
    return _require_strategy_prompt_response(result)


@mcp.tool()
def submit_prompt_revision(
    rationale: str,
    evidence_memory_ids: list[str],
    changes: list[dict[str, Any]],
) -> dict[str, Any]:
    """Submit one evidence-backed review revision batch for eligible prompts.

    Call this no more than once per Review run. Each ``changes`` item must be
    an object with integer ``target_sub_agent_id`` and ``base_revision_id``,
    plus string ``prompt`` fields. Do not use ``new_prompt``. If it fails,
    record the recommendation and error in the review memory instead of
    retrying.
    """
    _require_nonblank("rationale", rationale)
    if not isinstance(evidence_memory_ids, list) or not all(
        isinstance(value, str) and value.strip() for value in evidence_memory_ids
    ):
        raise ValueError("evidence_memory_ids must contain nonblank IDs")
    if not isinstance(changes, list) or not changes:
        raise ValueError("changes must be a non-empty list")
    result = _request(
        "POST",
        "/api/v1/strategy-prompts/revisions",
        json_body={
            "rationale": rationale,
            "evidence_memory_ids": evidence_memory_ids,
            "changes": changes,
        },
    )
    if not isinstance(result, dict) or set(result) != {"batch_id"} or not isinstance(result["batch_id"], int):
        raise RuntimeError("HyperVibes prompt revision returned unexpected shape")
    return result


@mcp.tool()
def list_analysis_instruments() -> list[str]:
    """List the selected active analysis instrument IDs for this agent.

    Call this before creating or updating an indicator and pass returned IDs
    unchanged as instrument_ids. If the list is empty, ask the operator to
    select analysis instruments in Settings; do not guess IDs or create.
    """
    result = _request("GET", "/api/v1/analysis-instruments")
    if not isinstance(result, list) or not all(isinstance(value, str) for value in result):
        raise RuntimeError("HyperVibes analysis instruments returned unexpected shape")
    return result


@mcp.tool()
def list_trading_instruments() -> list[str]:
    """List the active instruments currently allowed for new Trading exposure."""
    result = _request("GET", "/api/v1/trading-instruments")
    if not isinstance(result, list) or not all(isinstance(value, str) for value in result):
        raise RuntimeError("HyperVibes trading instruments returned unexpected shape")
    return result


@mcp.tool()
def set_trading_instrument_enabled(instrument_id: str, enabled: bool) -> list[str]:
    """Enable or disable one Trading instrument and return the resulting allowlist.

    Enabling is permitted only for an active instrument currently selected for
    Analysis. Disabling the final Trading instrument pauses new exposure but
    does not close positions.
    """
    if not isinstance(enabled, bool):
        raise ValueError("enabled must be a boolean")
    result = _request(
        "PUT",
        f"/api/v1/trading-instruments/{_require_nonblank('instrument_id', instrument_id)}",
        json_body={"enabled": enabled},
    )
    if not isinstance(result, list) or not all(isinstance(value, str) for value in result):
        raise RuntimeError("HyperVibes trading instruments returned unexpected shape")
    return result


@mcp.tool()
def list_indicators() -> list[dict[str, Any]]:
    """List this agent's indicator definitions and their latest run status."""
    result = _request("GET", "/api/v1/indicators")
    if not isinstance(result, list):
        raise RuntimeError("HyperVibes indicators returned unexpected shape")
    return result


@mcp.tool()
def get_indicator(indicator_id: str) -> dict[str, Any]:
    """Return an indicator's active source, inputs, and selected targets."""
    result = _request("GET", f"/api/v1/indicators/{_require_indicator_id(indicator_id)}")
    if not isinstance(result, dict):
        raise RuntimeError("HyperVibes indicator returned unexpected shape")
    return result


@mcp.tool()
def get_indicator_results(
    indicator_id: str,
    instrument_id: str | None = None,
    timeframe: str | None = None,
    limit: int | None = None,
) -> list[dict[str, Any]]:
    """Return bounded indicator runs, values, and diagnostics."""
    params: dict[str, Any] = {}
    if instrument_id is not None:
        params["instrument_id"] = _require_nonblank("instrument_id", instrument_id)
    if timeframe is not None:
        params["timeframe"] = _require_nonblank("timeframe", timeframe)
    if limit is not None:
        params["limit"] = _require_limit(limit)
    result = _request(
        "GET",
        f"/api/v1/indicators/{_require_indicator_id(indicator_id)}/results",
        params=params or None,
    )
    if not isinstance(result, list):
        raise RuntimeError("HyperVibes indicator results returned unexpected shape")
    return result


@mcp.tool()
def create_indicator(
    name: str,
    timeframe: str,
    instrument_ids: list[str],
    source: str,
    description: str = "",
    input_values: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Create a validated server-side PineScript indicator.

    First call list_analysis_instruments and use its returned IDs unchanged.
    Input values are keyed by each Pine input's title, not its variable name.
    """
    if not isinstance(description, str):
        raise ValueError("description must be a string")
    body = _require_indicator_fields(name, timeframe, instrument_ids, source, input_values)
    body["description"] = description
    result = _request("POST", "/api/v1/indicators", json_body=body)
    if not isinstance(result, dict):
        raise RuntimeError("HyperVibes indicator creation returned unexpected shape")
    return result


@mcp.tool()
def update_indicator(
    indicator_id: str,
    expected_version_id: str,
    name: str,
    timeframe: str,
    instrument_ids: list[str],
    source: str,
    enabled: bool = True,
    description: str = "",
    input_values: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Create and activate a new immutable PineScript indicator version.

    First call list_analysis_instruments and use its returned IDs unchanged.
    Input values are keyed by each Pine input's title, not its variable name.
    """
    if not isinstance(enabled, bool):
        raise ValueError("enabled must be a boolean")
    if not isinstance(description, str):
        raise ValueError("description must be a string")
    body = _require_indicator_fields(name, timeframe, instrument_ids, source, input_values)
    body.update(
        {
            "expected_version_id": _require_nonblank(
                "expected_version_id", expected_version_id
            ),
            "enabled": enabled,
            "description": description,
        }
    )
    result = _request(
        "PUT",
        f"/api/v1/indicators/{_require_indicator_id(indicator_id)}",
        json_body=body,
    )
    if not isinstance(result, dict):
        raise RuntimeError("HyperVibes indicator update returned unexpected shape")
    return result


@mcp.tool()
def get_trading_context(instrument_id: str) -> dict[str, Any]:
    """Return current analysis evidence and analyst status for one instrument."""
    instrument_id = _require_nonblank("instrument_id", instrument_id)
    result = _request(
        "GET",
        "/api/v1/memories/trading-context",
        params={"instrument_id": instrument_id},
    )
    if not isinstance(result, dict):
        raise RuntimeError("HyperVibes trading context returned unexpected shape")
    return result


@mcp.tool()
def get_memory_detail(memory_id: str) -> dict[str, Any]:
    """Return one agent-owned memory and its links."""
    memory_id = _require_nonblank("memory_id", memory_id)
    result = _request(
        "GET",
        f"/api/v1/memories/{memory_id}",
        params={"include": "links"},
    )
    if not isinstance(result, dict):
        raise RuntimeError("HyperVibes memory detail returned unexpected shape")
    return result


@mcp.tool()
def list_memories(
    scope_kind: str | None = None,
    instrument_id: str | None = None,
    timeframe: str | None = None,
    memory_type: str | None = None,
    since: str | None = None,
    until: str | None = None,
    limit: int | None = None,
    include_expired: bool = False,
) -> list[dict[str, Any]]:
    """List memory rows visible to this agent.

    All filters are optional. ``include_expired`` defaults to ``False`` to
    match the backend's default staleness hiding.

    Use ``scope_kind="agent"`` for agent-wide records. ``agent_learnings`` is
    a separate durable learning-memory type.
    """
    params: dict[str, Any] = {}
    if scope_kind is not None:
        if scope_kind not in {"agent", "instruments"}:
            raise ValueError("scope_kind must be agent or instruments")
        params["scope_kind"] = scope_kind
    if instrument_id is not None:
        params["instrument_id"] = _require_nonblank("instrument_id", instrument_id)
    if timeframe is not None:
        params["timeframe"] = _require_nonblank("timeframe", timeframe)
    if memory_type is not None:
        params["memory_type"] = _require_nonblank("memory_type", memory_type)
    if since is not None:
        params["since"] = _require_nonblank("since", since)
    if until is not None:
        params["until"] = _require_nonblank("until", until)
    if limit is not None:
        params["limit"] = _require_limit(limit)
    if include_expired:
        params["include_expired"] = "true"
    result = _request("GET", "/api/v1/memories", params=params or None)
    if not isinstance(result, list):
        raise RuntimeError("HyperVibes /memories returned unexpected shape")
    return result


@mcp.tool()
def list_orders(
    status: str | None = None,
    symbol: str | None = None,
    since: str | None = None,
    until: str | None = None,
    limit: int | None = None,
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
    if since is not None:
        params["since"] = _require_nonblank("since", since)
    if until is not None:
        params["until"] = _require_nonblank("until", until)
    if limit is not None:
        params["limit"] = _require_limit(limit)
    result = _request("GET", "/api/v1/orders", params=params or None)
    if not isinstance(result, list):
        raise RuntimeError("HyperVibes /orders returned unexpected shape")
    return result


@mcp.tool()
def list_account_transactions(
    since: str,
    until: str,
    symbol: str | None = None,
    event_category: str | None = None,
    limit: int | None = None,
    offset: int | None = None,
) -> list[dict[str, Any]]:
    """List durable Hyperliquid fills, funding, and ledger events for this agent.

    ``since`` and ``until`` are required RFC 3339 bounds. Results come from
    HyperVibes's account journal, not a direct exchange request. Use
    ``offset`` with a fixed ``limit`` to page through a review window until a
    page returns fewer rows than the requested limit.
    """
    params: dict[str, Any] = {
        "since": _require_nonblank("since", since),
        "until": _require_nonblank("until", until),
    }
    if symbol is not None:
        params["symbol"] = _require_nonblank("symbol", symbol)
    if event_category is not None:
        params["event_category"] = _require_nonblank(
            "event_category", event_category
        )
    if limit is not None:
        params["limit"] = _require_limit(limit)
    if offset is not None:
        params["offset"] = _require_offset(offset)
    result = _request("GET", "/api/v1/account/transactions", params=params)
    if not isinstance(result, list):
        raise RuntimeError(
            "HyperVibes /account/transactions returned unexpected shape"
        )
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
        raise RuntimeError("HyperVibes /orders/{id} returned unexpected shape")
    return result


@mcp.tool()
def write_memory(
    scope_kind: str,
    memory_type: str,
    summary: str,
    content: str,
    instrument_ids: list[str] | None = None,
    timeframe: str | None = None,
    metadata: dict[str, Any] | None = None,
    links: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    """Persist a memory for this agent.

    Required: ``scope_kind``, ``memory_type``, ``summary``, ``content``.
    Use ``scope_kind="agent"`` with no instrument IDs for agent-wide records,
    or ``scope_kind="instruments"`` with one or more unique canonical IDs.
    ``timeframe`` is optional. For a general memory, omit the ``timeframe``
    argument entirely; do not pass an empty string. ``metadata`` must be a
    JSON object when provided; ``None`` is stored as an empty object. ``links``
    may be a list of objects with ``target_memory_id``, ``link_type``, and an
    optional object ``metadata``. Run and sub-agent provenance is stamped by
    the server; do not include it in metadata.
    """
    if scope_kind not in {"agent", "instruments"}:
        raise ValueError("scope_kind must be agent or instruments")
    if instrument_ids is None:
        instrument_ids = []
    if not isinstance(instrument_ids, list) or not all(
        isinstance(instrument_id, str) and instrument_id.strip()
        for instrument_id in instrument_ids
    ):
        raise ValueError("instrument_ids must contain nonblank IDs")
    if len(set(instrument_ids)) != len(instrument_ids):
        raise ValueError("instrument_ids must not contain duplicates")
    if scope_kind == "agent" and instrument_ids:
        raise ValueError("instrument_ids must be empty for agent scope")
    if scope_kind == "instruments" and not instrument_ids:
        raise ValueError("instrument_ids is required for instruments scope")
    memory_type = _require_nonblank("memory_type", memory_type)
    summary = _require_nonblank("summary", summary)
    content = _require_nonblank("content", content)
    if timeframe is not None:
        timeframe = _require_nonblank("timeframe", timeframe)
    if metadata is not None and not isinstance(metadata, dict):
        raise ValueError("metadata must be a JSON object")
    if metadata is not None and {
        "source_run_id",
        "source_sub_agent_id",
        "source_sub_agent_key",
    }.intersection(metadata):
        raise ValueError("metadata must not include source run or sub-agent provenance")
    if links is not None:
        if not isinstance(links, list):
            raise ValueError("links must be a list")
        for index, link in enumerate(links):
            if not isinstance(link, dict):
                raise ValueError(f"links[{index}] must be a JSON object")
    body: dict[str, Any] = {
        "scope_kind": scope_kind,
        "instrument_ids": instrument_ids,
        "memory_type": memory_type,
        "summary": summary,
        "content": content,
    }
    if timeframe is not None:
        body["timeframe"] = timeframe
    body["metadata"] = metadata if metadata is not None else {}
    if links is not None:
        body["links"] = links
    result = _request("POST", "/api/v1/memories", json_body=body)
    if not isinstance(result, dict):
        raise RuntimeError("HyperVibes /memories POST returned unexpected shape")
    return result


@mcp.tool()
def submit_orders(
    orders: Annotated[
        list[SubmitOrderInput],
        Field(min_length=1, description="One or more validated order requests."),
    ],
) -> dict[str, Any]:
    """Submit one or more orders through the HyperVibes backend.

    For a limit order use ``symbol``, ``side``, ``order_type="limit"``,
    ``size``, and ``price``. For example:
    ``{"symbol":"ETH","side":"buy","order_type":"limit","size":0.004,"price":2606,"time_in_force":"gtc"}``.
    Opening agent orders should include the relevant trading-decision memory ID
    in ``memory_record_ids``. This is a real backend action: the server selects
    instruments, signs the request, and submits to Hyperliquid. The MCP server
    does not hold or use any private key.
    """
    if not isinstance(orders, list) or not orders:
        raise ValueError("orders must be a non-empty list")
    for index, order in enumerate(orders):
        if not isinstance(order, SubmitOrderInput):
            raise ValueError(f"orders[{index}] must be a validated order input")
    result = _request(
        "POST",
        "/api/v1/orders",
        json_body={"orders": [order.model_dump(mode="json") for order in orders]},
    )
    if not isinstance(result, dict):
        raise RuntimeError("HyperVibes /orders POST returned unexpected shape")
    return result


@mcp.tool()
def cancel_orders(
    orders: Annotated[
        list[CancelOrderInput],
        Field(min_length=1, description="One or more validated cancellation requests."),
    ],
) -> list[dict[str, Any]]:
    """Cancel one or more orders through the HyperVibes backend.

    Each item requires ``symbol`` and numeric ``oid``. For example:
    ``{"symbol":"ETH","oid":549220142898}``. Returns one outcome per
    requested cancel.
    """
    if not isinstance(orders, list) or not orders:
        raise ValueError("orders must be a non-empty list")
    for index, order in enumerate(orders):
        if not isinstance(order, CancelOrderInput):
            raise ValueError(f"orders[{index}] must be a validated cancellation input")
    result = _request(
        "POST",
        "/api/v1/orders/cancel",
        json_body={"orders": [order.model_dump(mode="json") for order in orders]},
    )
    if not isinstance(result, list):
        raise RuntimeError(
            "HyperVibes /orders/cancel returned unexpected shape"
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
            "HyperVibes /orders/cancel-all returned unexpected shape"
        )
    return result


NOTIFICATION_SEVERITIES = {"info", "warning", "error"}


def _require_severity(severity: str) -> str:
    if not isinstance(severity, str) or severity not in NOTIFICATION_SEVERITIES:
        allowed = ", ".join(sorted(NOTIFICATION_SEVERITIES))
        raise ValueError(f"severity must be one of: {allowed}")
    return severity


@mcp.tool()
def send_notification(title: str, body: str, severity: str = "info") -> dict[str, Any]:
    """Send a notification through the agent's configured messaging gateway.

    Args:
        title: Short notification title.
        body: Detailed notification body (plain text).
        severity: One of "info", "warning", "error". Affects gateway
            formatting (for example, a warning emoji prefix on Telegram).
    """
    title = _require_nonblank("title", title)
    body = _require_nonblank("body", body)
    severity = _require_severity(severity)
    result = _request(
        "POST",
        "/api/v1/notifications",
        json_body={
            "title": title,
            "body": body,
            "severity": severity,
        },
    )
    if not isinstance(result, dict) or not isinstance(result.get("id"), str):
        raise RuntimeError("HyperVibes /notifications returned unexpected shape")
    return result


def main() -> None:
    # Fail fast on missing config so OpenCode gets a clear stderr message
    # instead of a half-initialised server.
    _redirect_stderr_to_container_log()
    _, _, agent_key = _require_config()
    LOGGER.info("hypervibes_mcp_started agent_key=%s", agent_key)
    try:
        mcp.run()
    except BaseException as exc:
        if _is_broken_stdio_shutdown(exc):
            LOGGER.info("hypervibes_mcp_stopped reason=stdio_client_disconnected")
            return
        raise


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:  # noqa: BLE001
        print(f"hypervibes MCP server failed to start: {exc}", file=sys.stderr)
        raise

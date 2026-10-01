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
import math
import os
import re
import sys
from datetime import datetime, timedelta, timezone
from urllib.parse import quote
from decimal import Decimal
from pathlib import Path
from typing import Annotated, Any, Literal

import httpx
from dotenv import load_dotenv
from mcp.server.fastmcp import FastMCP
from pydantic import BaseModel, ConfigDict, Field, StrictInt, field_validator, model_validator
from pydantic_core import to_json


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
    return _indicator_filter("indicator_id", indicator_id)


def _indicator_filter(name: str, value: str) -> str:
    if not isinstance(value, str) or not value.strip() or len(value.encode("utf-8")) > 255:
        raise ValueError(f"{name} must be a nonblank string of at most 255 bytes")
    return value


def _require_indicator_fields(
    name: str,
    timeframes: list[str],
    instrument_ids: list[str],
    source: str,
    input_values: dict[str, Any] | None,
) -> dict[str, Any]:
    _require_nonblank("name", name)
    if not isinstance(timeframes, list) or not 1 <= len(timeframes) <= 8 or not all(
        isinstance(timeframe, str) and timeframe.strip() for timeframe in timeframes
    ):
        raise ValueError("timeframes must contain between 1 and 8 nonblank values")
    timeframes = [timeframe.strip() for timeframe in timeframes]
    if len(set(timeframes)) != len(timeframes):
        raise ValueError("timeframes must not contain duplicates")
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
        "timeframes": timeframes,
        "instrument_ids": instrument_ids,
        "source": source,
        "input_values": input_values if input_values is not None else {},
    }


INDICATOR_SCHEMA_VERSION = 2
INDICATOR_TEXT_BYTES = 24 * 1024
INDICATOR_TEXT_LINES = 1000


def _indicator_text(value: Any) -> str:
    """Mirror FastMCP 1.28.1 + OpenCode 1.18.32 text serialization.

    FastMCP uses pydantic_core.to_json(indent=2), one block per list item;
    OpenCode joins text blocks with two newlines. Structured content is not
    consumed as tool text. Keep this paired with test_indicator_transport.py.
    """
    items = value if isinstance(value, list) else [value]
    return "\n\n".join(to_json(item, indent=2).decode() for item in items)


def _indicator_text_size(value: Any) -> tuple[int, int]:
    text = _indicator_text(value)
    return len(text.encode("utf-8")), len(text.split("\n"))


def _indicator_fits(value: Any) -> bool:
    size, lines = _indicator_text_size(value)
    return size <= INDICATOR_TEXT_BYTES and lines <= INDICATOR_TEXT_LINES


def _log_indicator_output(tool: str, value: Any, reduced: bool) -> None:
    size, lines = _indicator_text_size(value)
    runs = value if isinstance(value, list) else []
    LOGGER.info(
        "hypervibes_mcp_output tool=%s schema_version=%s bytes=%s lines=%s "
        "bars=%s markers=%s definitions=%s budget_reduced=%s",
        tool, INDICATOR_SCHEMA_VERSION, size, lines,
        sum(len(run["bars"]) for run in runs),
        sum(len(run["markers"]) for run in runs),
        len(value["items"]) if isinstance(value, dict) else 0, reduced,
    )


def _indicator_integer(name: str, value: Any, minimum: int, maximum: int | None = None) -> int:
    if type(value) is not int or value < minimum or (maximum is not None and value > maximum):
        bounds = f"{minimum}..{maximum}" if maximum is not None else f">= {minimum}"
        raise ValueError(f"{name} must be an integer {bounds}")
    return value


def _indicator_number(value: Any, *, nullable: bool = False) -> bool:
    return (nullable and value is None) or (
        type(value) in (int, float) and math.isfinite(value)
    )


def _indicator_close(value: Any) -> bool:
    if type(value) in (int, float):
        return _indicator_number(value)
    if isinstance(value, str):
        try:
            return Decimal(value).is_finite()
        except ArithmeticError:
            return False
    return False


def _indicator_values(value: Any) -> dict[str, Any] | None:
    if value is None:
        return None
    if not isinstance(value, dict) or not all(
        isinstance(name, str) and _indicator_number(number, nullable=True)
        for name, number in value.items()
    ):
        raise RuntimeError("Malformed indicator latest values; inspect the stored run in the operator UI")
    return {name: number for name, number in value.items()}


def _indicator_run_header(value: dict[str, Any]) -> dict[str, Any]:
    fields = (
        "id", "indicator_definition_id", "indicator_version_id", "instrument_id",
        "timeframe", "scheduled_for", "status",
    )
    if not all(isinstance(value.get(field), str) and value[field].strip() for field in fields):
        raise RuntimeError("Malformed indicator run provenance; inspect the stored run in the operator UI")
    return {"schema_version": INDICATOR_SCHEMA_VERSION, **{field: value[field] for field in fields}}


def _indicator_diagnostics(value: dict[str, Any]) -> dict[str, Any]:
    diagnostics = value.get("diagnostics", [])
    if not isinstance(diagnostics, list):
        raise RuntimeError("Malformed indicator diagnostics; inspect the stored run in the operator UI")
    projected = []
    abbreviated = len(diagnostics) > 3
    for diagnostic in diagnostics[:3]:
        if not isinstance(diagnostic, dict):
            raise RuntimeError("Malformed indicator diagnostic; inspect the stored run in the operator UI")
        item = {}
        for field in ("severity", "message"):
            text = diagnostic.get(field, "")
            if not isinstance(text, str):
                raise RuntimeError("Malformed indicator diagnostic text; inspect the stored run in the operator UI")
            shortened = text.encode("utf-8")[:512].decode("utf-8", errors="ignore")
            abbreviated |= shortened != text
            item[field] = shortened
        for field in ("line", "column"):
            number = diagnostic.get(field)
            if number is not None and (type(number) is not int or number < 0):
                raise RuntimeError("Malformed indicator diagnostic location; inspect the stored run in the operator UI")
            item[field] = number
        projected.append(item)
    error = value.get("error_summary")
    if error is not None and not isinstance(error, str):
        raise RuntimeError("Malformed indicator error summary; inspect the stored run in the operator UI")
    shortened_error = error.encode("utf-8")[:512].decode("utf-8", errors="ignore") if error else error
    return {
        "diagnostics": projected, "diagnostic_count": len(diagnostics),
        "diagnostics_abbreviated": abbreviated, "error_summary": shortened_error,
        "error_summary_abbreviated": shortened_error != error,
    }


def _indicator_page_metadata(run: dict[str, Any]) -> None:
    """Recompute cursors after removing complete units, using actual counts."""
    start, count = run["bar_start"], len(run["bars"])
    run["bar_returned_count"] = count
    run["bar_end"] = start + count
    run["previous_bar_start"] = max(0, start - max(1, count)) if start > 0 else None
    # Exclusive end supports gap-free backward paging even if the next page
    # fits fewer bars. bar_start is always a forward, oldest-first range.
    run["previous_bar_end"] = start if start > 0 else None
    run["next_bar_start"] = start + count if start + count < (run["bar_count"] or 0) else None
    run["bars_complete"] = run["evidence_available"] and start == 0 and count == run["bar_count"]
    start, count = run["marker_start"], len(run["markers"])
    run["marker_returned_count"] = count
    run["previous_marker_start"] = max(0, start - max(1, count)) if start > 0 else None
    run["next_marker_start"] = start + count if start + count < (run["marker_count"] or 0) else None
    run["markers_complete"] = run["evidence_available"] and start == 0 and count == run["marker_count"]


def _indicator_run_for_agent(
    value: dict[str, Any], bar_start: int | None = None, bar_limit: int = 20,
    marker_start: int = 0, marker_limit: int = 20, bar_end: int | None = None,
) -> dict[str, Any]:
    run = _indicator_run_header(value)
    run["latest_values"] = _indicator_values(value.get("latest_values"))
    run.update(_indicator_diagnostics(value))
    candles, plots, visual = (value.get(field) for field in ("candle_data", "plot_data", "visual_data"))
    available = run["status"] == "succeeded" and all(data is not None for data in (candles, plots, visual))
    run.update({
        "evidence_available": available, "bar_count": None, "bar_start": 0, "bars": [],
        "marker_count": None, "marker_start": 0, "markers": [],
        "marker_order": "source_bar_desc_event_position_asc", "budget_reduced": False,
    })
    if not available:
        _indicator_page_metadata(run)
        return run
    if not isinstance(candles, list) or not all(
        isinstance(candle, dict) and isinstance(candle.get("opened_at"), str)
        and candle["opened_at"].strip() and _indicator_close(candle.get("close"))
        for candle in candles
    ):
        raise RuntimeError("Malformed indicator candle data; inspect the stored run in the operator UI")
    candle_count = len(candles)
    if not isinstance(plots, dict) or not all(
        isinstance(name, str) and isinstance(series, list) and len(series) == candle_count
        and all(_indicator_number(number, nullable=True) for number in series)
        for name, series in plots.items()
    ):
        raise RuntimeError("Malformed or unaligned indicator plot data; inspect the stored run in the operator UI")
    if not isinstance(visual, dict) or not isinstance(visual.get("markers"), list):
        raise RuntimeError("Malformed indicator visual data; inspect the stored run in the operator UI")
    markers = visual["markers"]
    for marker in markers:
        if not isinstance(marker, dict):
            raise RuntimeError("Malformed indicator marker; inspect the stored run in the operator UI")
        kind, index, offset = (marker.get(field) for field in ("kind", "bar_index", "offset"))
        strings = ["title"] + (["text"] if kind != "plotarrow" else []) + (["character"] if kind == "plotchar" else [])
        if (
            kind not in ("plotshape", "plotchar", "plotarrow") or type(index) is not int
            or not 0 <= index < candle_count or type(offset) is not int
            or not -2000 <= offset <= 2000 or not _indicator_number(marker.get("value"))
            or not all(isinstance(marker.get(field), str) for field in strings)
        ):
            raise RuntimeError("Malformed or unaligned indicator marker; inspect the stored run in the operator UI")
    if bar_start is not None and bar_start > candle_count:
        raise ValueError("bar_start exceeds this exact run's bar_count")
    if bar_end is not None and bar_end > candle_count:
        raise ValueError("bar_end exceeds this exact run's bar_count")
    if marker_start > len(markers):
        raise ValueError("marker_start exceeds this exact run's marker_count")
    end = bar_end if bar_end is not None else candle_count
    start = bar_start if bar_start is not None else max(0, end - bar_limit)
    end = min(end, start + bar_limit)
    run["bar_count"], run["bar_start"] = candle_count, start
    def source_times(index: int) -> dict[str, str]:
        opened_at = candles[index]["opened_at"]
        try:
            opened = datetime.fromisoformat(opened_at.replace("Z", "+00:00"))
            interval = re.fullmatch(r"([1-9][0-9]*)([mhd])", run["timeframe"])
            if opened.tzinfo is None or interval is None:
                raise ValueError("invalid candle provenance")
            seconds = int(interval[1]) * {"m": 60, "h": 3600, "d": 86400}[interval[2]]
            closed = opened + timedelta(seconds=seconds)
        except (ValueError, OverflowError) as exc:
            raise RuntimeError("Malformed indicator candle time or timeframe; inspect the stored run in the operator UI") from exc
        return {"opened_at": opened_at, "closed_at": closed.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")}

    run["bars"] = [
        {"bar_index": index, **source_times(index), "close": candles[index]["close"],
         **{field: candles[index][field] for field in ("open", "high", "low", "volume") if field in candles[index]},
         "plots": {name: series[index] for name, series in plots.items()}}
        for index in range(start, end)
    ]
    # Python's stable sort preserves original order for duplicate same-bar
    # events; event_position exposes their immutable identity to consumers.
    ordered = sorted(enumerate(markers), key=lambda event: -event[1]["bar_index"])
    run["marker_count"], run["marker_start"] = len(markers), marker_start
    run["markers"] = [
        {"event_position": position, **source_times(marker["bar_index"]),
         **{field: marker[field] for field in (
             ["bar_index", "kind", "title", "value", "offset"]
             + (["text"] if marker["kind"] != "plotarrow" else [])
             + (["character"] if marker["kind"] == "plotchar" else [])
         )}}
        for position, marker in ordered[marker_start:marker_start + marker_limit]
    ]
    _indicator_page_metadata(run)
    return run


def _fit_indicator_runs(runs: list[dict[str, Any]], tail_bars: bool) -> list[dict[str, Any]]:
    # Fail aggregate/header overflow before repeatedly fitting large requested
    # pages. Every nonempty requested stream must retain at least one unit.
    minimum = []
    for run in runs:
        smallest = {**run, "bars": run["bars"][-1:] if tail_bars else run["bars"][:1],
                    "markers": run["markers"][:1]}
        if tail_bars and run["bars"]:
            smallest["bar_start"] = run["bars"][-1]["bar_index"]
        smallest["budget_reduced"] = len(run["bars"]) > 1 or len(run["markers"]) > 1
        _indicator_page_metadata(smallest)
        minimum.append(smallest)
    if not _indicator_fits(minimum):
        raise ValueError(
            "Indicator response cannot fit the text budget. Use a smaller run limit or exact run_id; "
            "if one run still fails, an oversized header/bar/event needs operator inspection."
        )
    reduced = False
    while not _indicator_fits(runs):
        candidates = [
            (len(to_json(run[field], indent=2)), index, field)
            for index, run in enumerate(runs) for field in ("bars", "markers")
            if len(run[field]) > 1
        ]
        if not candidates:
            raise RuntimeError("Indicator fitting could not make progress; request a smaller exact-run page")
        _, index, field = max(candidates)
        run = runs[index]
        if field == "bars" and tail_bars:
            run[field].pop(0)
            run["bar_start"] += 1
        else:
            run[field].pop()
        run["budget_reduced"] = reduced = True
        _indicator_page_metadata(run)
    _log_indicator_output("get_indicator_results", runs, reduced)
    return runs


def _indicator_list_item_for_agent(value: Any) -> dict[str, Any]:
    if (
        not isinstance(value, dict)
        or not all(isinstance(value.get(field), str) and value[field].strip() for field in ("id", "name"))
        or not isinstance(value.get("enabled"), bool)
        or not isinstance(value.get("timeframes"), list)
        or not all(isinstance(timeframe, str) for timeframe in value["timeframes"])
        or (value.get("active_version_id") is not None and not isinstance(value["active_version_id"], str))
    ):
        raise RuntimeError("HyperVibes indicators returned unexpected shape")
    item = {field: value[field] for field in (
        "id", "name", "enabled", "timeframes", "active_version_id"
    ) if field in value}
    # The authenticated discovery endpoint excludes archived definitions.
    item["archived"] = False
    latest_run = value.get("latest_run")
    if latest_run is not None:
        if not isinstance(latest_run, dict):
            raise RuntimeError("HyperVibes latest indicator run returned unexpected shape")
        item["latest_run"] = _indicator_run_header(latest_run)
    else:
        item["latest_run"] = None
    return item


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
def list_indicators(limit: StrictInt = 20, offset: StrictInt = 0) -> dict[str, Any]:
    """Discover definitions and compact latest-run headers, without histories.

    Returns items, total, offset, returned_count, next_offset. Follow next_offset
    for additional definitions. latest_run is only one instrument/timeframe;
    inspect every relevant frozen target with get_indicator_results explicitly.
    """
    _indicator_integer("limit", limit, 1, 100)
    _indicator_integer("offset", offset, 0)
    result = _request("GET", "/api/v1/indicators")
    if not isinstance(result, list):
        raise RuntimeError("HyperVibes indicators returned unexpected shape")
    if offset > len(result):
        raise ValueError("offset exceeds indicator total; restart discovery")
    envelope: dict[str, Any] = {
        "schema_version": INDICATOR_SCHEMA_VERSION, "items": [], "total": len(result),
        "offset": offset, "returned_count": 0, "next_offset": None, "budget_reduced": False,
    }
    for value in result[offset:offset + limit]:
        envelope["items"].append(_indicator_list_item_for_agent(value))
        count = len(envelope["items"])
        envelope["returned_count"] = count
        envelope["next_offset"] = offset + count if offset + count < len(result) else None
        if not _indicator_fits(envelope):
            envelope["items"].pop()
            if not envelope["items"]:
                raise ValueError("Indicator discovery item exceeds the text budget; use get_indicator for operator detail inspection")
            count -= 1
            envelope.update(returned_count=count, next_offset=offset + count, budget_reduced=True)
            break
    _log_indicator_output("list_indicators", envelope, envelope["budget_reduced"])
    return envelope


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
    timeframe: str,
    instrument_id: str | None = None,
    run_id: str | None = None,
    limit: StrictInt | None = None,
    bar_start: StrictInt | None = None,
    bar_limit: StrictInt = 20,
    marker_start: StrictInt | None = None,
    marker_limit: StrictInt = 20,
    bar_end: StrictInt | None = None,
) -> list[dict[str, Any]]:
    """Read compact exact-run evidence: latest 20 bars and newest 20 markers.

    bar_limit and marker_limit (1..100) are maximum requested counts; the whole
    call may fit fewer complete units. Markers page independently, newest source
    candle first, then original event_position. opened_at is source candle time;
    offset is visual only. Empty unavailable evidence is not absence of signals.
    Check evidence_available, bars_complete, markers_complete and actual counts.
    Run limit defaults to 1. Every offset requires run_id from the first page:
    pass bar_start=next_bar_start for forward history, or the exclusive
    bar_end=previous_bar_end for gap-free backward history; do not combine them.
    Pass marker_start=next_marker_start independently to read older events.
    """
    for name, value in (("bar_start", bar_start), ("marker_start", marker_start), ("bar_end", bar_end)):
        if value is not None:
            _indicator_integer(name, value, 0)
            if run_id is None:
                raise ValueError("Indicator page offsets require an exact run_id from the first result page")
    if bar_start is not None and bar_end is not None:
        raise ValueError("Use either bar_start (forward) or bar_end (backward), not both")
    _indicator_integer("bar_limit", bar_limit, 1, 100)
    _indicator_integer("marker_limit", marker_limit, 1, 100)
    params: dict[str, Any] = {}
    if instrument_id is not None:
        params["instrument_id"] = _indicator_filter("instrument_id", instrument_id)
    params["timeframe"] = _indicator_filter("timeframe", timeframe)
    if run_id is not None:
        params["run_id"] = _indicator_filter("run_id", run_id)
    params["limit"] = _indicator_integer("limit", limit if limit is not None else 1, 1, 100)
    result = _request(
        "GET",
        f"/api/v1/indicators/{_require_indicator_id(indicator_id)}/results",
        params=params or None,
    )
    if not isinstance(result, list) or not all(isinstance(run, dict) for run in result):
        raise RuntimeError("HyperVibes indicator results returned unexpected shape")
    return _fit_indicator_runs([
        _indicator_run_for_agent(run, bar_start, bar_limit, marker_start or 0, marker_limit, bar_end)
        for run in result
    ], tail_bars=bar_start is None)


@mcp.tool()
def create_indicator(
    name: str,
    timeframes: list[str],
    instrument_ids: list[str],
    source: str,
    description: str = "",
    input_values: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Create a validated server-side PineScript indicator using pine-indicators guidance.

    First call list_analysis_instruments and use its returned IDs unchanged.
    Input values are keyed by each Pine input's title, not its variable name.
    One input-value set is executed independently for every configured timeframe.
    Numeric plot and plotshape, plotchar, and plotarrow marker outputs are supported.
    """
    if not isinstance(description, str):
        raise ValueError("description must be a string")
    body = _require_indicator_fields(name, timeframes, instrument_ids, source, input_values)
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
    timeframes: list[str],
    instrument_ids: list[str],
    source: str,
    enabled: bool = True,
    description: str = "",
    input_values: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Create and activate a new immutable PineScript version using pine-indicators guidance.

    First call list_analysis_instruments and use its returned IDs unchanged.
    Input values are keyed by each Pine input's title, not its variable name.
    Inspect prior numeric plots and marker events before making evidence-based revisions.
    """
    if not isinstance(enabled, bool):
        raise ValueError("enabled must be a boolean")
    if not isinstance(description, str):
        raise ValueError("description must be a string")
    body = _require_indicator_fields(name, timeframes, instrument_ids, source, input_values)
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
def list_account_trades(limit: int | None = None, offset: int | None = None) -> list[dict[str, Any]]:
    """List flat-to-flat perpetual trade cycles, including open/incomplete cycles."""
    params: dict[str, Any] = {}
    if limit is not None:
        params["limit"] = _require_limit(limit)
        if limit > 100:
            raise ValueError("trade limit must be <= 100")
    if offset is not None:
        params["offset"] = _require_offset(offset)
    result = _request("GET", "/api/v1/account/trades", params=params or None)
    if not isinstance(result, list):
        raise RuntimeError("HyperVibes /account/trades returned unexpected shape")
    return result


@mcp.tool()
def get_account_trade(trade_id: str) -> dict[str, Any]:
    """Get a trade cycle with its allocated entry and exit fills."""
    from uuid import UUID
    trade_id = str(UUID(_require_nonblank("trade_id", trade_id)))
    result = _request("GET", f"/api/v1/account/trades/{trade_id}")
    if not isinstance(result, dict):
        raise RuntimeError("HyperVibes /account/trades/{id} returned unexpected shape")
    return result


def _journal_path(kind: str, target: str) -> str:
    if kind not in ("trade", "fill", "funding", "ledger"):
        raise ValueError("kind must be trade, fill, funding, or ledger")
    target = _require_nonblank("target", target)
    if "/" in target or len(target) > 300:
        raise ValueError("invalid journal target")
    return f"/api/v1/account/journal/{kind}/{quote(target, safe='')}/notes"


@mcp.tool()
def list_journal_notes(kind: str, target: str) -> list[dict[str, Any]]:
    """List chronological attributed notes on a trade or account activity event."""
    result = _request("GET", _journal_path(kind, target))
    if not isinstance(result, list):
        raise RuntimeError("HyperVibes journal notes returned unexpected shape")
    return result


@mcp.tool()
def add_journal_note(kind: str, target: str, body: str) -> dict[str, Any]:
    """Append an attributed note to a trade or activity event (Review or permitted Chat)."""
    body = _require_nonblank("body", body).strip()
    if not body or len(body.encode("utf-8")) > 4000:
        raise ValueError("note must be 1..4000 bytes")
    result = _request("POST", _journal_path(kind, target), json_body={"body": body})
    if not isinstance(result, dict):
        raise RuntimeError("HyperVibes journal note returned unexpected shape")
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
    Copy target IDs unchanged. A definitive 422 ``invalid_memory_link`` includes
    a zero-based ``link_index``; refetch authorized context, repair only that
    reference and retry at most once. Do not guess IDs, drop required links, or
    retry ambiguous failures (writes have no idempotency key). Corrections use
    ``link_type="corrects"`` to reference the exact original memory.
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

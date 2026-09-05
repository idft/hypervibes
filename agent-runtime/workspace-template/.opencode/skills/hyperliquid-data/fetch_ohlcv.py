#!/usr/bin/env python3
"""Fetch OHLCV candles from Hyperliquid's public REST API."""

import argparse
import json
import os
import sys
import time
import uuid
from pathlib import Path
from typing import Any

import requests

INTERVAL_MS = {
    "1m": 60_000,
    "3m": 180_000,
    "5m": 300_000,
    "15m": 900_000,
    "30m": 1_800_000,
    "1h": 3_600_000,
    "2h": 7_200_000,
    "4h": 14_400_000,
    "8h": 28_800_000,
    "12h": 43_200_000,
    "1d": 86_400_000,
    "3d": 259_200_000,
    "1w": 604_800_000,
    "1M": 2_592_000_000,
}

DEFAULT_OUTPUT_DIR = Path("scratch/ohlcv")
MAX_FETCH_ATTEMPTS = 8


def base_url() -> str:
    env = os.environ.get("HYPERLIQUID_ENVIRONMENT", "mainnet").strip().lower()
    if env == "testnet":
        return "https://api.hyperliquid-testnet.xyz"
    return "https://api.hyperliquid.xyz"


def candle_start_ms(candle: dict[str, Any]) -> int:
    """Return the candle's open/start timestamp as an epoch millisecond.

    Hyperliquid's ``candleSnapshot`` returns objects whose ``t`` field is the
    candle's start timestamp. Price and volume fields are numeric strings.
    """
    if not isinstance(candle, dict) or "t" not in candle:
        raise RuntimeError("unexpected Hyperliquid candle shape")
    raw = candle["t"]
    if isinstance(raw, str):
        return int(float(raw))
    return int(raw)


def candle_close_ms(candle: dict[str, Any], interval_ms: int) -> int:
    """Return the candle's close timestamp.

    The Hyperliquid ``candleSnapshot`` endpoint omits a dedicated close
    column: a candle that *opened* at ``T`` for an interval ``interval_ms``
    has its close at ``T + interval_ms`` (exclusive of the next interval's
    open). We derive close from the start timestamp plus the requested
    interval rather than trusting any heuristics against the API's
    unpredictable optional `` endTime`` fields.
    """
    return candle_start_ms(candle) + interval_ms


def normalize_candle(candle: dict[str, Any]) -> dict[str, int | float]:
    """Convert one native Hyperliquid candle into the canonical input shape."""
    try:
        return {
            "timestamp_ms": candle_start_ms(candle),
            "open": float(candle["o"]),
            "high": float(candle["h"]),
            "low": float(candle["l"]),
            "close": float(candle["c"]),
            "volume": float(candle["v"]),
        }
    except (KeyError, TypeError, ValueError) as exc:
        raise RuntimeError("unexpected Hyperliquid candle shape") from exc


def canonical_payload(
    symbol: str,
    timeframe: str,
    interval_ms: int,
    candles: list[dict[str, Any]],
) -> dict[str, Any]:
    """Build the stable input envelope consumed by Python analysis scripts."""
    return {
        "symbol": symbol,
        "timeframe": timeframe,
        "interval_ms": interval_ms,
        "candles": [normalize_candle(candle) for candle in candles],
    }


def fetch_ohlcv(
    symbol: str,
    timeframe: str,
    *,
    limit: int | None = None,
    start_time: int | None = None,
    end_time: int | None = None,
) -> list[dict[str, Any]]:
    if timeframe not in INTERVAL_MS:
        raise RuntimeError(f"unsupported timeframe '{timeframe}'")

    interval_ms = INTERVAL_MS[timeframe]

    if end_time is None:
        end_time = int(time.time() * 1000)
    if start_time is None:
        if limit is None:
            limit = 200
        start_time = max(0, end_time - (limit * interval_ms))

    payload = {
        "type": "candleSnapshot",
        "req": {
            "coin": symbol,
            "interval": timeframe,
            "startTime": start_time,
            "endTime": end_time,
        },
    }

    try:
        response = requests.post(f"{base_url()}/info", json=payload, timeout=30)
        response.raise_for_status()
    except requests.RequestException as exc:
        raise RuntimeError(f"failed to fetch Hyperliquid candles: {exc}") from exc

    data = response.json()
    if not isinstance(data, list) or not all(isinstance(candle, dict) for candle in data):
        raise RuntimeError("unexpected candle response shape")
    return data


def filter_closed_before(
    candles: list[dict[str, Any]], interval_ms: int, closed_before_ms: int
) -> list[dict[str, Any]]:
    """Return only candles that are fully closed at or before the boundary.

    A candle is "fully closed" when its close timestamp (start + interval_ms)
    is less than or equal to ``closed_before_ms``. Candles that merely *open*
    before the boundary but have not yet closed are excluded; this is the
    behaviour that callers such as a 15-minute sub-agent fetching 1-hour data at
    a half-hour boundary require (the still-open 1-hour candle would
    otherwise leak future data into the analysis).
    """
    return [c for c in candles if candle_close_ms(c, interval_ms) <= closed_before_ms]


def maximum_close_ms(candles: list[dict[str, Any]], interval_ms: int) -> int | None:
    """Return the maximum close timestamp of ``candles`` or ``None`` if empty."""
    if not candles:
        return None
    return max(candle_close_ms(c, interval_ms) for c in candles)


def output_dir_from_argument(value: str) -> Path:
    """Resolve a caller-selected directory confined to this run's scratch tree."""
    requested = Path(value)
    if requested.is_absolute():
        raise RuntimeError("--output-dir must be relative to the workspace scratch directory")

    scratch_root = (Path.cwd() / "scratch").resolve()
    output_dir = (Path.cwd() / requested).resolve()
    try:
        output_dir.relative_to(scratch_root)
    except ValueError as exc:
        raise RuntimeError("--output-dir must remain under scratch/") from exc
    return output_dir


def write_candles(output_dir: Path, payload: dict[str, Any]) -> Path:
    output_dir.mkdir(parents=True, exist_ok=True)

    timestamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    output_path = output_dir / f"{timestamp}-{uuid.uuid4().hex[:12]}.json"
    temp_path = output_path.with_name(f".{output_path.name}.tmp")

    with temp_path.open("w", encoding="utf-8") as f:
        json.dump(payload, f, separators=(",", ":"))
        f.write("\n")
    os.replace(temp_path, output_path)
    return output_path


def fetch_closed_ohlcv(
    symbol: str,
    timeframe: str,
    *,
    limit: int,
    start_time: int | None,
    closed_before_ms: int,
) -> list[dict[str, Any]]:
    """Fetch at least ``limit`` eligible candles, expanding the request window.

    Hyperliquid can include a candle that is open at the requested boundary, so
    fetch limits are only hints. The local close-time filter is authoritative.
    """
    interval_ms = INTERVAL_MS[timeframe]
    window_candles = limit + 1
    for _ in range(MAX_FETCH_ATTEMPTS):
        requested_start = (
            start_time
            if start_time is not None
            else max(0, closed_before_ms - window_candles * interval_ms)
        )
        candles = fetch_ohlcv(
            symbol,
            timeframe,
            start_time=requested_start,
            end_time=closed_before_ms,
        )
        closed = filter_closed_before(candles, interval_ms, closed_before_ms)
        closed.sort(key=candle_start_ms)
        if len(closed) >= limit:
            return closed[-limit:]
        if start_time is not None or requested_start == 0:
            break
        window_candles *= 2
    raise RuntimeError(
        f"fewer than {limit} candles closed at or before {closed_before_ms} were available"
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Fetch Hyperliquid OHLCV candles")
    parser.add_argument("symbol", help="Coin symbol, e.g. BTC")
    parser.add_argument("timeframe", help="Candle interval, e.g. 15m")
    parser.add_argument("--limit", type=int, help="Number of candles to fetch")
    parser.add_argument("--start-time", type=int, help="Start time in epoch milliseconds")
    parser.add_argument("--end-time", type=int, help="End time in epoch milliseconds")
    parser.add_argument(
        "--closed-before",
        type=int,
        help=(
            "Restrict returned candles to those that fully closed at or before "
            "this epoch millisecond boundary. Filtering is applied "
            "internally using candle close time (start + interval), so candles "
            "that merely opened before the boundary but have not closed yet "
            "are excluded."
        ),
    )
    parser.add_argument(
        "--output-dir",
        default=DEFAULT_OUTPUT_DIR.as_posix(),
        help="Run-local output directory beneath scratch/ (default: scratch/ohlcv)",
    )
    parser.add_argument(
        "--stdout",
        action="store_true",
        help="Print full candle JSON to stdout instead of writing an output file",
    )
    args = parser.parse_args()

    if args.timeframe not in INTERVAL_MS:
        parser.error(f"unsupported timeframe {args.timeframe!r}")
    if args.limit is not None and args.limit <= 0:
        parser.error("--limit must be greater than zero")

    interval_ms = INTERVAL_MS[args.timeframe]
    closed_before_ms = args.closed_before
    limit: int | None = None

    if closed_before_ms is not None:
        if closed_before_ms <= 0:
            parser.error("--closed-before must be greater than zero")
        if args.end_time is not None:
            parser.error("--closed-before cannot be combined with --end-time")
        limit = 200 if args.limit is None else args.limit
        if not isinstance(limit, int):
            parser.error("--limit must be an integer")
        end_time = closed_before_ms
    else:
        end_time = args.end_time

    try:
        if closed_before_ms is not None:
            if limit is None:
                raise RuntimeError("closed-candle fetch limit was not initialized")
            candles = fetch_closed_ohlcv(
                args.symbol,
                args.timeframe,
                limit=limit,
                start_time=args.start_time,
                closed_before_ms=closed_before_ms,
            )
        else:
            candles = fetch_ohlcv(
                args.symbol,
                args.timeframe,
                limit=args.limit,
                start_time=args.start_time,
                end_time=end_time,
            )
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(1)

    requested_boundary_ms = closed_before_ms
    actual_max_close_ms = maximum_close_ms(candles, interval_ms)
    try:
        payload = canonical_payload(args.symbol, args.timeframe, interval_ms, candles)
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(1)

    if args.stdout:
        print(json.dumps(payload, indent=2))
        return

    try:
        output_path = write_candles(output_dir_from_argument(args.output_dir), payload)
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(1)
    except OSError as exc:
        print(f"failed to write OHLCV output file: {exc}", file=sys.stderr)
        sys.exit(1)

    manifest = {
        "symbol": args.symbol,
        "timeframe": args.timeframe,
        "interval_ms": interval_ms,
        "candles": len(candles),
        "output_path": output_path.relative_to(Path.cwd().resolve()).as_posix(),
        "requested_boundary_ms": requested_boundary_ms,
        "actual_max_close_ms": actual_max_close_ms,
    }
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()

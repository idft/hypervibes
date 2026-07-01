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
    "5m": 300_000,
    "15m": 900_000,
    "30m": 1_800_000,
    "1h": 3_600_000,
    "2h": 7_200_000,
    "4h": 14_400_000,
    "1d": 86_400_000,
}

DEFAULT_TTL_HOURS = 24.0
CACHE_ROOT = Path("scratch/ohlcv-cache")


def base_url() -> str:
    env = os.environ.get("HYPERLIQUID_ENVIRONMENT", "mainnet").strip().lower()
    if env == "testnet":
        return "https://api.hyperliquid-testnet.xyz"
    return "https://api.hyperliquid.xyz"


def fetch_ohlcv(
    symbol: str,
    timeframe: str,
    *,
    limit: int | None = None,
    start_time: int | None = None,
    end_time: int | None = None,
) -> list[Any]:
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
    if not isinstance(data, list):
        raise RuntimeError("unexpected candle response shape")
    return data


def safe_path_component(value: str) -> str:
    safe = "".join(ch if ch.isalnum() or ch in "._:@-" else "_" for ch in value.strip())
    return safe or "unknown"


def cache_dir_for(symbol: str, timeframe: str) -> Path:
    return CACHE_ROOT / safe_path_component(symbol) / safe_path_component(timeframe)


def write_cached_candles(symbol: str, timeframe: str, candles: list[Any]) -> Path:
    cache_dir = cache_dir_for(symbol, timeframe)
    cache_dir.mkdir(parents=True, exist_ok=True)

    timestamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    output_path = cache_dir / f"{timestamp}-{uuid.uuid4().hex[:12]}.json"
    temp_path = output_path.with_name(f".{output_path.name}.tmp")

    with temp_path.open("w", encoding="utf-8") as f:
        json.dump(candles, f, separators=(",", ":"))
        f.write("\n")
    os.replace(temp_path, output_path)
    return output_path


def prune_cache_dir(symbol: str, timeframe: str, ttl_hours: float, keep_path: Path | None) -> int:
    if ttl_hours < 0:
        raise RuntimeError("--ttl-hours must be non-negative")

    cache_dir = cache_dir_for(symbol, timeframe)
    if not cache_dir.exists():
        return 0

    cutoff = time.time() - (ttl_hours * 3600)
    removed = 0
    keep_resolved = keep_path.resolve() if keep_path is not None and keep_path.exists() else None

    for path in cache_dir.glob("*.json"):
        try:
            if keep_resolved is not None and path.resolve() == keep_resolved:
                continue
            if path.stat().st_mtime >= cutoff:
                continue
            path.unlink()
            removed += 1
        except OSError:
            # Best-effort cleanup; do not fail the data fetch because a concurrent
            # process moved or deleted a cache file first.
            continue
    return removed


def main() -> None:
    parser = argparse.ArgumentParser(description="Fetch Hyperliquid OHLCV candles")
    parser.add_argument("symbol", help="Coin symbol, e.g. BTC")
    parser.add_argument("timeframe", help="Candle interval, e.g. 15m")
    parser.add_argument("--limit", type=int, help="Number of candles to fetch")
    parser.add_argument("--start-time", type=int, help="Start time in epoch milliseconds")
    parser.add_argument("--end-time", type=int, help="End time in epoch milliseconds")
    parser.add_argument(
        "--ttl-hours",
        type=float,
        default=DEFAULT_TTL_HOURS,
        help="Delete cached files for this symbol/timeframe older than this many hours",
    )
    parser.add_argument(
        "--no-prune",
        action="store_true",
        help="Do not prune old cache files for this symbol/timeframe",
    )
    parser.add_argument(
        "--stdout",
        action="store_true",
        help="Print full candle JSON to stdout instead of writing a cache file",
    )
    args = parser.parse_args()

    if args.ttl_hours < 0:
        parser.error("--ttl-hours must be non-negative")

    try:
        candles = fetch_ohlcv(
            args.symbol,
            args.timeframe,
            limit=args.limit,
            start_time=args.start_time,
            end_time=args.end_time,
        )
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(1)

    if args.stdout:
        if not args.no_prune:
            prune_cache_dir(args.symbol, args.timeframe, args.ttl_hours, None)
        print(json.dumps(candles, indent=2))
        return

    try:
        output_path = write_cached_candles(args.symbol, args.timeframe, candles)
        pruned_files = 0
        if not args.no_prune:
            pruned_files = prune_cache_dir(args.symbol, args.timeframe, args.ttl_hours, output_path)
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(1)
    except OSError as exc:
        print(f"failed to write OHLCV cache file: {exc}", file=sys.stderr)
        sys.exit(1)

    manifest = {
        "symbol": args.symbol,
        "timeframe": args.timeframe,
        "candles": len(candles),
        "output_path": output_path.as_posix(),
        "ttl_hours": args.ttl_hours,
        "pruned_files": pruned_files,
    }
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()

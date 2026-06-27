#!/usr/bin/env python3
"""Fetch OHLCV candles from Hyperliquid's public REST API."""

import argparse
import json
import os
import sys
import time
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


def main() -> None:
    parser = argparse.ArgumentParser(description="Fetch Hyperliquid OHLCV candles")
    parser.add_argument("symbol", help="Coin symbol, e.g. BTC")
    parser.add_argument("timeframe", help="Candle interval, e.g. 15m")
    parser.add_argument("--limit", type=int, help="Number of candles to fetch")
    parser.add_argument("--start-time", type=int, help="Start time in epoch milliseconds")
    parser.add_argument("--end-time", type=int, help="End time in epoch milliseconds")
    args = parser.parse_args()

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

    print(json.dumps(candles, indent=2))


if __name__ == "__main__":
    main()

from __future__ import annotations

import os
import time

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
}


def _base_url() -> str:
    environment = os.environ.get("HYPERLIQUID_ENVIRONMENT", "mainnet").strip().lower()
    if environment == "testnet":
        return "https://api.hyperliquid-testnet.xyz"
    return "https://api.hyperliquid.xyz"


def fetch_ohlcv(
    symbol: str,
    timeframe: str,
    *,
    limit: int = 200,
    start_time: int | None = None,
    end_time: int | None = None,
) -> list[dict]:
    if timeframe not in INTERVAL_MS:
        raise RuntimeError(f"unsupported timeframe '{timeframe}'")

    if end_time is None:
        end_time = int(time.time() * 1000)
    if start_time is None:
        start_time = max(0, end_time - (limit * INTERVAL_MS[timeframe]))

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
        response = requests.post(f"{_base_url()}/info", json=payload, timeout=30)
        response.raise_for_status()
    except requests.RequestException as exc:
        raise RuntimeError(f"failed to fetch Hyperliquid candles: {exc}") from exc

    data = response.json()
    if not isinstance(data, list):
        raise RuntimeError("unexpected candle response shape")
    return data

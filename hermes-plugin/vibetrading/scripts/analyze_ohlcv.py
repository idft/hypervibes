#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib

from _common import add_plugin_root_to_path, parse_json, print_error, print_json

add_plugin_root_to_path()

analyze_ohlcv = importlib.import_module("vibetrading.py.analysis").analyze_ohlcv


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--candles-json", required=True)
    parser.add_argument("--indicators", default="sma,ema,rsi,macd")
    args = parser.parse_args()

    try:
        candles = parse_json(args.candles_json, label="candles JSON")
        indicators = [item.strip() for item in args.indicators.split(",") if item.strip()]
        print_json(analyze_ohlcv(candles, indicators))
        return 0
    except Exception as exc:
        print_error(str(exc))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

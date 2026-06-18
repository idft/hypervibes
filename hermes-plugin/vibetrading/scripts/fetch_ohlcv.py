#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib

from _common import add_plugin_root_to_path, print_error, print_json

add_plugin_root_to_path()

fetch_ohlcv = importlib.import_module("vibetrading.py.hyperliquid_data").fetch_ohlcv


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("symbol")
    parser.add_argument("timeframe")
    parser.add_argument("--limit", type=int, default=200)
    parser.add_argument("--start-time", type=int)
    parser.add_argument("--end-time", type=int)
    args = parser.parse_args()

    try:
        print_json(
            fetch_ohlcv(
                args.symbol,
                args.timeframe,
                limit=args.limit,
                start_time=args.start_time,
                end_time=args.end_time,
            )
        )
        return 0
    except Exception as exc:
        print_error(str(exc))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

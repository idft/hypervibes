#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib

from _common import add_plugin_root_to_path, print_error, print_json

add_plugin_root_to_path()

list_memories = importlib.import_module("vibetrading.py.memories").list_memories


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--symbol")
    parser.add_argument("--timeframe")
    parser.add_argument("--memory-type")
    parser.add_argument("--since")
    parser.add_argument("--until")
    parser.add_argument("--limit", type=int)
    args = parser.parse_args()

    try:
        print_json(
            list_memories(
                symbol=args.symbol,
                timeframe=args.timeframe,
                memory_type=args.memory_type,
                since=args.since,
                until=args.until,
                limit=args.limit,
            )
        )
        return 0
    except Exception as exc:
        print_error(str(exc))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

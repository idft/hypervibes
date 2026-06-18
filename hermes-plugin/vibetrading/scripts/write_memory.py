#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib

from _common import add_plugin_root_to_path, parse_json, print_error, print_json

add_plugin_root_to_path()

write_memory = importlib.import_module("vibetrading.py.memories").write_memory


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--symbol", required=True)
    parser.add_argument("--memory-type", required=True)
    parser.add_argument("--summary", required=True)
    parser.add_argument("--content", required=True)
    parser.add_argument("--timeframe")
    parser.add_argument("--metadata-json", default="{}")
    args = parser.parse_args()

    try:
        print_json(
            write_memory(
                symbol=args.symbol,
                memory_type=args.memory_type,
                summary=args.summary,
                content=args.content,
                timeframe=args.timeframe,
                metadata=parse_json(args.metadata_json, label="metadata JSON"),
            )
        )
        return 0
    except Exception as exc:
        print_error(str(exc))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

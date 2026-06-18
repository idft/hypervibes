#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib

from _common import add_plugin_root_to_path, print_error, print_json

add_plugin_root_to_path()

list_orders = importlib.import_module("vibetrading.py.orders").list_orders


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--status")
    parser.add_argument("--symbol")
    args = parser.parse_args()

    try:
        print_json(list_orders(status=args.status, symbol=args.symbol))
        return 0
    except Exception as exc:
        print_error(str(exc))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

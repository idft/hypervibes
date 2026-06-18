#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib

from _common import add_plugin_root_to_path, parse_json, print_error, print_json

add_plugin_root_to_path()

cancel_orders = importlib.import_module("vibetrading.py.orders").cancel_orders


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--orders-json", required=True)
    args = parser.parse_args()

    try:
        print_json(cancel_orders(parse_json(args.orders_json, label="orders JSON")))
        return 0
    except Exception as exc:
        print_error(str(exc))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

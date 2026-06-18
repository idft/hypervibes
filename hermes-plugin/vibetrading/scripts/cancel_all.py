#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib

from _common import add_plugin_root_to_path, print_error, print_json

add_plugin_root_to_path()

cancel_all = importlib.import_module("vibetrading.py.orders").cancel_all


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--symbol")
    args = parser.parse_args()

    try:
        print_json(cancel_all(symbol=args.symbol))
        return 0
    except Exception as exc:
        print_error(str(exc))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

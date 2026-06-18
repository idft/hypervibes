#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib

from _common import add_plugin_root_to_path, print_error, print_json

add_plugin_root_to_path()

get_job_context = importlib.import_module("vibetrading.py.job_context").get_job_context


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--job-kind", choices=["analysis", "trading"], required=True)
    args = parser.parse_args()

    try:
        print_json(get_job_context(args.job_kind))
        return 0
    except Exception as exc:
        print_error(str(exc))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

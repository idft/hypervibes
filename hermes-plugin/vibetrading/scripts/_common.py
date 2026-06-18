from __future__ import annotations

import json
import sys
from pathlib import Path


def add_plugin_root_to_path() -> None:
    plugin_root = Path(__file__).resolve().parents[1]
    package_parent = plugin_root.parent
    if str(package_parent) not in sys.path:
        sys.path.insert(0, str(package_parent))


def print_json(data) -> None:
    json.dump(data, sys.stdout, indent=2)
    sys.stdout.write("\n")


def print_error(message: str) -> None:
    sys.stderr.write(f"{message}\n")


def parse_json(value: str, *, label: str):
    try:
        return json.loads(value)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"invalid {label}: {exc}") from exc

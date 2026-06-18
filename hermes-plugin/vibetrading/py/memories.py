from __future__ import annotations

from typing import Any

from .client import VibetradingClient


def list_memories(**kwargs: Any) -> list[dict[str, Any]]:
    return VibetradingClient().list_memories(**kwargs)


def write_memory(**kwargs: Any) -> dict[str, Any]:
    return VibetradingClient().write_memory(**kwargs)

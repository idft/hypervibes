from __future__ import annotations

from .client import VibetradingClient


def get_account() -> dict:
    return VibetradingClient().get_account()

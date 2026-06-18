from __future__ import annotations

from .client import VibetradingClient


def list_orders(status: str | None = None, symbol: str | None = None) -> list[dict]:
    return VibetradingClient().list_orders(status=status, symbol=symbol)


def place_orders(orders: list[dict]) -> list[dict]:
    return VibetradingClient().place_orders(orders)


def cancel_orders(orders: list[dict]) -> list[dict]:
    return VibetradingClient().cancel_orders(orders)


def cancel_all(symbol: str | None = None) -> dict:
    return VibetradingClient().cancel_all(symbol=symbol)

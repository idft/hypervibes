from __future__ import annotations

from typing import Any


def _float(value: Any, label: str) -> float:
    if value is None:
        raise RuntimeError(f"candle is missing {label}")
    return float(value)


def _normalize_candles(candles: list[Any]) -> list[dict[str, float | int]]:
    rows: list[dict[str, float | int]] = []
    for candle in candles:
        if isinstance(candle, dict):
            rows.append(
                {
                    "timestamp": int(candle.get("t") or candle.get("time") or candle.get("timestamp") or 0),
                    "open": _float(candle.get("o") or candle.get("open"), "open"),
                    "high": _float(candle.get("h") or candle.get("high"), "high"),
                    "low": _float(candle.get("l") or candle.get("low"), "low"),
                    "close": _float(candle.get("c") or candle.get("close"), "close"),
                    "volume": float(candle.get("v") or candle.get("volume") or 0),
                }
            )
        elif isinstance(candle, (list, tuple)) and len(candle) >= 6:
            rows.append(
                {
                    "timestamp": int(candle[0]),
                    "open": float(candle[1]),
                    "high": float(candle[2]),
                    "low": float(candle[3]),
                    "close": float(candle[4]),
                    "volume": float(candle[5]),
                }
            )
    if not rows:
        raise RuntimeError("no candles provided")
    return sorted(rows, key=lambda row: int(row["timestamp"]))


def _round(value: float | None, precision: int = 4) -> float | None:
    if value is None:
        return None
    return round(value, precision)


def _sma(values: list[float], period: int) -> float | None:
    if len(values) < period:
        return None
    window = values[-period:]
    return sum(window) / period


def _ema_series(values: list[float], period: int) -> list[float]:
    if not values:
        return []
    multiplier = 2 / (period + 1)
    ema = [values[0]]
    for value in values[1:]:
        ema.append((value - ema[-1]) * multiplier + ema[-1])
    return ema


def _rsi(values: list[float], period: int = 14) -> float | None:
    if len(values) <= period:
        return None
    gains: list[float] = []
    losses: list[float] = []
    for previous, current in zip(values[:-1], values[1:]):
        delta = current - previous
        gains.append(max(delta, 0.0))
        losses.append(max(-delta, 0.0))
    avg_gain = sum(gains[-period:]) / period
    avg_loss = sum(losses[-period:]) / period
    if avg_loss == 0:
        return 100.0
    rs = avg_gain / avg_loss
    return 100 - (100 / (1 + rs))


def analyze_ohlcv(candles: list[Any], indicators: list[str] | None = None) -> dict[str, Any]:
    rows = _normalize_candles(candles)
    closes = [float(row["close"]) for row in rows]
    indicators = indicators or ["sma", "ema", "rsi", "macd"]
    results: dict[str, Any] = {
        "candle_count": len(rows),
        "latest_timestamp": int(rows[-1]["timestamp"]),
        "latest_close": _round(closes[-1]),
        "indicators": {},
    }

    if "sma" in indicators:
        results["indicators"]["sma_20"] = _round(_sma(closes, 20))
    if "ema" in indicators:
        ema20 = _ema_series(closes, 20)
        results["indicators"]["ema_20"] = _round(ema20[-1] if ema20 else None)
    if "rsi" in indicators:
        results["indicators"]["rsi_14"] = _round(_rsi(closes), 2)
    if "macd" in indicators:
        ema12 = _ema_series(closes, 12)
        ema26 = _ema_series(closes, 26)
        macd_series = [fast - slow for fast, slow in zip(ema12, ema26)]
        signal_series = _ema_series(macd_series, 9)
        macd_value = macd_series[-1] if macd_series else None
        signal_value = signal_series[-1] if signal_series else None
        histogram = None
        if macd_value is not None and signal_value is not None:
            histogram = macd_value - signal_value
        results["indicators"]["macd"] = {
            "line": _round(macd_value),
            "signal": _round(signal_value),
            "histogram": _round(histogram),
        }

    return results

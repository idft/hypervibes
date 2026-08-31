"""Tests for the Hyperliquid ``fetch_ohlcv.py`` helper.

These tests focus on the interval-aware boundary filtering that
``--closed-before`` performs so analysis paths can rely on never seeing
an open candle that started before the boundary but has not closed yet.
"""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[5]
SCRIPT = (
    REPO_ROOT
    / "agent-runtime"
    / "workspace-template"
    / ".opencode"
    / "skills"
    / "hyperliquid-data"
    / "fetch_ohlcv.py"
)


def _load_module() -> Any:
    spec = importlib.util.spec_from_file_location("fetch_ohlcv", SCRIPT)
    assert spec and spec.loader
    module: Any = importlib.util.module_from_spec(spec)
    sys.modules["fetch_ohlcv"] = module
    spec.loader.exec_module(module)
    return module


def _candle(start_ms: int, *, close_price: float = 1.0) -> dict[str, Any]:
    """Build a minimal native Hyperliquid candleSnapshot object."""
    return {
        "t": start_ms,
        "T": start_ms + 899_999,
        "s": "BTC",
        "i": "15m",
        "o": "1.0",
        "h": "1.0",
        "l": "1.0",
        "c": str(close_price),
        "v": "0",
        "n": 1,
    }


class CandleCloseMsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.module = _load_module()

    def test_close_is_start_plus_interval(self) -> None:
        module = self.module
        m = _candle(1783960200000)
        self.assertEqual(module.candle_close_ms(m, 3_600_000), 1783963800000)
        self.assertEqual(module.candle_close_ms(m, 900_000), 1783961100000)

    def test_close_handles_numeric_string_start(self) -> None:
        module = self.module
        m = _candle(1783960200000)
        m["t"] = "1783960200000"
        self.assertEqual(module.candle_close_ms(m, 3_600_000), 1783963800000)

    def test_normalizes_native_hyperliquid_object(self) -> None:
        normalized = self.module.normalize_candle(_candle(123, close_price=2.5))
        self.assertEqual(
            normalized,
            {
                "timestamp_ms": 123,
                "open": 1.0,
                "high": 1.0,
                "low": 1.0,
                "close": 2.5,
                "volume": 0.0,
            },
        )

    def test_rejects_positional_candle_rows(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "unexpected Hyperliquid candle shape"):
            self.module.normalize_candle([123, "1", "1", "1", "1", "0", "0"])


class FilterClosedBeforeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.module = _load_module()

    def test_filters_open_candle_at_half_hour_boundary_for_hour_data(self) -> None:
        """A 15-minute sub-agent fetching 1-hour data at the half-hour boundary
        must exclude the still-open 1-hour candle that opened at the top
        of the hour. The 1-hour candle that opened at ``B - 1h`` closes
        exactly at ``B`` and is included by the ``close <= B`` rule."""
        module = self.module
        interval_ms = 3_600_000  # 1h
        boundary = 1_783_960_200_000  # arbitrary half-hour boundary
        candles = [
            _candle(boundary - 4 * interval_ms),  # closes 1h before boundary
            _candle(boundary - 3 * interval_ms),  # closes at boundary - 2h (still open by old code)
            _candle(boundary - interval_ms),      # closes AT boundary
            _candle(boundary),                    # closes 1h AFTER boundary
        ]

        kept = module.filter_closed_before(candles, interval_ms, boundary)

        # Candles that close at or before boundary survive.
        kept_starts = [module.candle_start_ms(c) for c in kept]
        self.assertEqual(
            kept_starts,
            [boundary - 4 * interval_ms, boundary - 3 * interval_ms, boundary - interval_ms],
        )
        # The candle that starts AT the boundary is excluded.
        self.assertNotIn(boundary, kept_starts)

    def test_filter_includes_candle_closing_exactly_at_boundary(self) -> None:
        module = self.module
        interval_ms = 3_600_000
        b = 1_784_000_000_000
        # Candle that opens at boundary - interval_ms closes exactly at boundary.
        edge = _candle(b - interval_ms)
        self.assertEqual(module.candle_close_ms(edge, interval_ms), b)
        self.assertEqual(module.filter_closed_before([edge], interval_ms, b), [edge])

    def test_filter_returns_empty_for_all_open_candles(self) -> None:
        module = self.module
        interval_ms = 3_600_000
        b = 1_784_000_000_000
        candles = [_candle(b), _candle(b + interval_ms)]
        self.assertEqual(module.filter_closed_before(candles, interval_ms, b), [])

    def test_filter_preserves_input_order(self) -> None:
        module = self.module
        interval_ms = 900_000  # 15m
        b = 1_784_000_000_000
        candles = [
            _candle(b - 5 * interval_ms),
            _candle(b - 4 * interval_ms),
            _candle(b - 3 * interval_ms),
        ]
        kept = module.filter_closed_before(candles, interval_ms, b)
        self.assertTrue(kept)
        self.assertEqual(
            [module.candle_start_ms(c) for c in kept],
            [b - 5 * interval_ms, b - 4 * interval_ms, b - 3 * interval_ms],
        )


class MaximumCloseMsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.module = _load_module()

    def test_returns_none_for_empty_list(self) -> None:
        self.assertIsNone(self.module.maximum_close_ms([], 3_600_000))

    def test_returns_max_close(self) -> None:
        module = self.module
        interval_ms = 900_000
        candles = [_candle(100), _candle(300), _candle(200)]
        self.assertEqual(module.maximum_close_ms(candles, interval_ms), 300 + interval_ms)


class ClosedFetchTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.module = _load_module()

    def test_expands_window_until_requested_eligible_count_is_available(self) -> None:
        module = self.module
        interval_ms = module.INTERVAL_MS["15m"]
        boundary = 1_784_000_000_000
        calls: list[int] = []

        def fake_fetch(symbol, timeframe, *, limit=None, start_time=None, end_time=None):
            assert end_time == boundary
            assert start_time is not None
            calls.append(start_time)
            if len(calls) == 1:
                return [_candle(boundary - interval_ms)]
            return [
                _candle(boundary - 3 * interval_ms),
                _candle(boundary - 2 * interval_ms),
                _candle(boundary - interval_ms),
                _candle(boundary),
            ]

        from unittest import mock

        with mock.patch.object(module, "fetch_ohlcv", side_effect=fake_fetch):
            candles = module.fetch_closed_ohlcv(
                "BTC", "15m", limit=3, start_time=None, closed_before_ms=boundary
            )

        self.assertEqual(len(calls), 2)
        self.assertEqual(
            [module.candle_start_ms(candle) for candle in candles],
            [boundary - 3 * interval_ms, boundary - 2 * interval_ms, boundary - interval_ms],
        )

    def test_rejects_insufficient_fixed_start_window(self) -> None:
        module = self.module
        interval_ms = module.INTERVAL_MS["15m"]
        boundary = 1_784_000_000_000

        from unittest import mock

        with mock.patch.object(
            module,
            "fetch_ohlcv",
            return_value=[_candle(boundary - interval_ms)],
        ):
            with self.assertRaisesRegex(RuntimeError, "fewer than 2 candles"):
                module.fetch_closed_ohlcv(
                    "BTC",
                    "15m",
                    limit=2,
                    start_time=boundary - 2 * interval_ms,
                    closed_before_ms=boundary,
                )


class OutputDirectoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.module = _load_module()

    def test_accepts_output_directory_beneath_scratch(self) -> None:
        import os
        import tempfile

        with tempfile.TemporaryDirectory() as temporary:
            original_cwd = os.getcwd()
            os.chdir(temporary)
            try:
                self.assertEqual(
                    self.module.output_dir_from_argument("scratch/ohlcv/custom"),
                    Path(temporary) / "scratch/ohlcv/custom",
                )
            finally:
                os.chdir(original_cwd)

    def test_rejects_output_directory_outside_scratch(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "under scratch"):
            self.module.output_dir_from_argument("../outside")


class ManifestIncludesBoundaryTests(unittest.TestCase):
    """Smoke-test that main() emits the requested boundary and max close
    in the manifest, exercising the close-filtering code path."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.module = _load_module()

    def _run_main(self, argv: list[str], fake_candles: list[Any]) -> dict[str, Any]:
        """Invoke `main` with mocked HTTP and isolated cwd."""
        import json, os, tempfile
        from unittest import mock

        module = self.module

        # `captured_end_time` is intentionally inspected to verify the
        # request shape during assertion of the manifest. We never read it
        # for assertions here, but keep it for future tests.
        captured_end_time: dict[str, int | None] = {}

        def fake_fetch_ohlcv(symbol, timeframe, *, limit=None, start_time=None, end_time=None):
            captured_end_time["end_time"] = end_time
            return list(fake_candles)

        with tempfile.TemporaryDirectory() as tmp:
            cwd = os.getcwd()
            os.chdir(tmp)
            try:
                with mock.patch.object(sys, "argv", argv):
                    with mock.patch.object(module, "fetch_ohlcv", side_effect=fake_fetch_ohlcv):
                        captured_stdout: dict[str, str] = {}

                        def fake_print(*args, **kwargs):
                            captured_stdout["out"] = args[0] if args else ""

                        with mock.patch("builtins.print", side_effect=fake_print):
                            try:
                                module.main()
                                manifest_text = captured_stdout.get("out", "")
                                manifest = json.loads(manifest_text)
                                output_path = manifest.get("output_path")
                                if output_path:
                                    manifest["_cached_payload"] = json.loads(
                                        Path(output_path).read_text()
                                    )
                                return manifest
                            except SystemExit as exc:
                                raise RuntimeError(
                                    f"main exited with {exc.code}; stdout={captured_stdout}"
                                ) from exc
            finally:
                os.chdir(cwd)

    def test_manifest_records_boundary_and_max_close_when_filtering(self) -> None:
        module = self.module
        interval_ms = 3_600_000
        b = 1_783_960_200_000
        # Candle that opened 3h before boundary and closes 2h before boundary.
        closed_candle = _candle(b - 3 * interval_ms)
        # Candle that opened AT the boundary - 1h closes AT the boundary
        # (must be filtered out).
        boundary_close_candle = _candle(b - interval_ms)
        # Candle that opens AT the boundary (closes 1h later).
        future_candle = _candle(b)

        manifest = self._run_main(
            ["fetch_ohlcv.py", "BTC", "1h", "--limit", "2", "--closed-before", str(b)],
            fake_candles=[closed_candle, boundary_close_candle, future_candle],
        )

        self.assertEqual(manifest["requested_boundary_ms"], b)
        self.assertEqual(manifest["timeframe"], "1h")
        self.assertEqual(manifest["interval_ms"], interval_ms)
        self.assertEqual(manifest["candles"], 2)
        self.assertEqual(manifest["actual_max_close_ms"], b)

    def test_manifest_records_null_boundary_without_closed_before(self) -> None:
        module = self.module
        candles = [_candle(100), _candle(200)]
        manifest = self._run_main(
            ["fetch_ohlcv.py", "BTC", "15m", "--limit", "2"],
            fake_candles=candles,
        )
        self.assertIsNone(manifest["requested_boundary_ms"])
        self.assertEqual(manifest["actual_max_close_ms"], 200 + 900_000)

    def test_cached_file_is_canonical_analyzer_input(self) -> None:
        manifest = self._run_main(
            ["fetch_ohlcv.py", "BTC", "2h", "--limit", "1"],
            fake_candles=[_candle(100, close_price=3.0)],
        )
        payload = manifest["_cached_payload"]
        self.assertEqual(payload["symbol"], "BTC")
        self.assertEqual(payload["timeframe"], "2h")
        self.assertEqual(payload["interval_ms"], 7_200_000)
        self.assertEqual(payload["candles"][0]["timestamp_ms"], 100)
        self.assertEqual(payload["candles"][0]["close"], 3.0)


if __name__ == "__main__":
    unittest.main()

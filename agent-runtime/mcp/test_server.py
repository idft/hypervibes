"""Lightweight tests for the HyperVibes MCP server helpers.

Unit tests use a fake FastMCP so they also run without the MCP package. When
the pinned package is available, a separate process checks the real low-level
transport serialization. Tests cover env loading, validation, authenticated
requests, indicator budgets and complete immutable history recovery.
"""

from __future__ import annotations

import importlib
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import types
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any
from unittest import mock

from pydantic import ValidationError
from pydantic_core import to_json

REPO_ROOT = Path(__file__).resolve().parents[2]
MCP_DIR = REPO_ROOT / "agent-runtime" / "mcp"
REAL_MCP_AVAILABLE = importlib.util.find_spec("mcp") is not None


def indicator_text(value: Any) -> str:
    """FastMCP 1.28.1 emits one pretty JSON text block per list item.

    OpenCode 1.18.32 joins blocks with two newlines, then applies independent
    50 KiB UTF-8 / 2,000 line limits (tool/truncate.ts, session/tools.ts).
    The real-package integration test checks this mirror against call_tool.
    """
    items = value if isinstance(value, list) else [value]
    return "\n\n".join(to_json(item, indent=2).decode() for item in items)


def source_open(index: int) -> str:
    return (datetime(2026, 9, 20, tzinfo=timezone.utc) + timedelta(minutes=15 * index)).isoformat().replace("+00:00", "Z")


def dense_indicator_run(bars: int = 500, plots: int = 3, events: int = 4) -> dict[str, Any]:
    candles = [{"opened_at": source_open(index), "close": str(index),
                "open": str(index), "high": str(index + 1), "low": str(index), "volume": "12"}
               for index in range(bars)]
    markers = []
    for index in range(bars):
        for position in range(events):
            kind = ("plotshape", "plotchar", "plotarrow")[position % 3]
            marker = {
                "kind": kind, "bar_index": index, "value": -2.5 if kind == "plotarrow" else 1.0,
                "title": ("LR", "SR", "LB", "SB")[position % 4],
                "offset": -1, "color": {"red": 255, "green": 128, "blue": 0, "transparency": 0},
                "location": "belowbar", "style": "labelup", "size": "small",
            }
            if kind != "plotarrow":
                marker["text"] = "候補" * 42 + "!" * 4  # Full permitted 256 UTF-8 bytes.
            if kind == "plotchar":
                marker["character"] = "↑"
            markers.append(marker)
    return {
        "id": "frozen-run-id", "agent_key": "default",
        "indicator_definition_id": "indicator-id", "indicator_version_id": "version-id",
        "instrument_id": "ETH", "timeframe": "15m", "scheduled_for": "2026-09-30T07:15:00Z",
        "status": "succeeded", "candle_data": candles,
        "plot_data": {f"Plot {n}": list(range(bars)) for n in range(plots)},
        "latest_values": {f"Plot {n}": bars - 1 for n in range(plots)},
        "visual_data": {"version": 1, "markers": markers}, "diagnostics": [],
        "error_summary": None,
    }


def indicator_definition(run: dict[str, Any], **fields: Any) -> dict[str, Any]:
    return {"id": "indicator-id", "name": "Entry", "enabled": True,
            "timeframes": ["15m"], "active_version_id": "version-id", "latest_run": run, **fields}


def _install_fake_mcp() -> None:
    """Inject a minimal fake of ``mcp.server.fastmcp`` so the module imports."""
    if "mcp" in sys.modules:
        return
    mcp_pkg = types.ModuleType("mcp")
    server_pkg = types.ModuleType("mcp.server")
    fastmcp_mod = types.ModuleType("mcp.server.fastmcp")

    class _FakeFastMCP:
        def __init__(self, name: str) -> None:
            self.name = name
            self.tools: dict[str, object] = {}

        def tool(self):
            def decorator(fn):
                self.tools[fn.__name__] = fn
                return fn

            return decorator

        def run(self) -> None:
            return None

    setattr(fastmcp_mod, "FastMCP", _FakeFastMCP)
    sys.modules["mcp"] = mcp_pkg
    sys.modules["mcp.server"] = server_pkg
    sys.modules["mcp.server.fastmcp"] = fastmcp_mod


def _load_server(defaults: dict[str, str] | None = None):
    _install_fake_mcp()
    # Ensure the module-level config load doesn't fail on import. The real
    # server reads the workspace `.env`; tests then override `CONFIG` to
    # drive specific behavior.
    prior: dict[str, str | None] = {}
    defaults = defaults or {}
    for key, value in defaults.items():
        prior[key] = os.environ.get(key)
        os.environ[key] = value
    try:
        spec = importlib.util.spec_from_file_location(
            "hypervibes_mcp_server", MCP_DIR / "server.py"
        )
        assert spec and spec.loader
        module: Any = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        spec.loader.exec_module(module)
        return module
    finally:
        for key, value in prior.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value


class HyperVibesMcpServerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.server = _load_server(
            {
                "HYPERVIBES_API_BASE_URL": "http://example.test",
                "HYPERVIBES_API_KEY": "vta_test_default",
                "HYPERVIBES_AGENT_KEY": "default",
            }
        )
        cls.server.LOGGER.setLevel("WARNING")

    def test_loads_required_env_from_workspace(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            workspace = Path(tmp)
            workspace.joinpath(".env").write_text(
                "HYPERVIBES_API_BASE_URL=http://example.test/\n"
                "HYPERVIBES_API_KEY=vta_test_xyz\n"
                "HYPERVIBES_AGENT_KEY=btc-2\n",
                encoding="utf-8",
            )
            with mock.patch.dict(os.environ, {}, clear=True):
                with mock.patch.object(self.server.Path, "cwd", return_value=workspace):
                    base_url, api_key, agent_key = self.server._load_config()
        self.assertEqual(base_url, "http://example.test")  # trailing slash stripped
        self.assertEqual(api_key, "vta_test_xyz")
        self.assertEqual(agent_key, "btc-2")

    def test_server_registers_the_non_coding_tool_set(self) -> None:
        registered = set(self.server.mcp.tools)
        self.assertTrue(
            {
                "get_account",
                "list_strategy_prompts",
                "get_strategy_prompt",
                "update_strategy_prompt",
                "list_analysis_instruments",
                "list_trading_instruments",
                "set_trading_instrument_enabled",
                "list_indicators",
                "get_indicator",
                "get_indicator_results",
                "create_indicator",
                "update_indicator",
                "get_trading_context",
                "get_memory_detail",
                "list_memories",
                "list_orders",
                "list_account_transactions",
                "get_order",
                "write_memory",
                "submit_orders",
                "cancel_orders",
                "cancel_all_orders",
                "send_notification",
            }.issubset(registered)
        )
        self.assertNotIn("get_latest_analysis", registered)
        self.assertNotIn("get_market_analysis", registered)
        self.assertNotIn("request_analysis_coding", registered)
        self.assertNotIn("request_coding", registered)
        self.assertNotIn("coding_validate_candidate", registered)
        self.assertNotIn("coding_submit_report", registered)

    def test_role_profiles_have_expected_memory_and_prompt_permissions(self) -> None:
        profiles = (
            Path(__file__).parents[2]
            / "agent-runtime"
            / "workspace-template"
            / ".opencode"
            / "agents"
        )
        for profile_name in ["analysis.md", "trading.md", "review.md", "agent-conversations.md"]:
            profile = (profiles / profile_name).read_text(encoding="utf-8")
            self.assertIn("hypervibes_*: deny", profile, profile_name)
            self.assertNotIn("coding", profile.lower(), profile_name)
            self.assertNotIn("analysis-coding", profile, profile_name)
            self.assertNotIn("python-analysis", profile, profile_name)
        for profile_name in ["analysis.md", "trading.md"]:
            profile = (profiles / profile_name).read_text(encoding="utf-8")
            self.assertNotIn("hypervibes_list_strategy_prompts:", profile, profile_name)
            self.assertNotIn("hypervibes_get_strategy_prompt:", profile, profile_name)
            self.assertNotIn("hypervibes_update_strategy_prompt:", profile, profile_name)
        trading = (profiles / "trading.md").read_text(encoding="utf-8")
        self.assertIn("hypervibes_get_trading_context: allow", trading)
        self.assertIn("hypervibes_write_memory: allow", trading)
        review = (profiles / "review.md").read_text(encoding="utf-8")
        self.assertIn("hypervibes_list_strategy_prompts: allow", review)
        self.assertIn("hypervibes_get_strategy_prompt: allow", review)
        self.assertIn("hypervibes_submit_prompt_revision: deny", review)
        self.assertIn("pine-indicators: allow", review)
        self.assertIn("load the `pine-indicators` skill", review)
        self.assertNotIn("hypervibes_update_strategy_prompt:", review)
        chat = (profiles / "agent-conversations.md").read_text(encoding="utf-8")
        self.assertIn("pine-indicators: allow", chat)
        self.assertIn("the `pine-indicators` skill", chat)
        analysis = (profiles / "analysis.md").read_text(encoding="utf-8")
        self.assertIn("numeric plots and marker events", analysis)
        skill = profiles.parent / "skills" / "pine-indicators" / "SKILL.md"
        self.assertTrue(skill.is_file())
        self.assertIn("plotshape()", skill.read_text(encoding="utf-8"))
        container_config = (
            Path(__file__).parents[2]
            / "agent-runtime"
            / "container"
            / "opencode.jsonc"
        ).read_text(encoding="utf-8")
        self.assertIn('"pine-indicators": "allow"', container_config)
        self.assertIn("load the pine-indicators skill", container_config)

    def test_indicator_reads_hide_internal_visual_data_versioning(self) -> None:
        run = dense_indicator_run(1, 1, 1)
        run.update(claim_token="secret-claim", lease_expires_at="date", next_attempt_at="date",
                   future_http_field="x" * 100_000)

        with mock.patch.object(self.server, "_request", return_value=[run]):
            results = self.server.get_indicator_results("indicator-id", "1h")
        self.assertNotIn("visual_data", results[0])
        self.assertNotIn("claim_token", results[0])
        self.assertNotIn("lease_expires_at", results[0])
        self.assertNotIn("next_attempt_at", results[0])
        self.assertNotIn("future_http_field", results[0])
        self.assertEqual(results[0]["markers"][0]["opened_at"], source_open(0))
        self.assertEqual(results[0]["markers"][0]["closed_at"], source_open(1))
        self.assertNotIn("color", results[0]["markers"][0])

        with mock.patch.object(
            self.server,
            "_request",
            return_value=[indicator_definition(run)],
        ):
            indicators = self.server.list_indicators()
        latest_run = indicators["items"][0]["latest_run"]
        self.assertNotIn("visual_data", latest_run)
        self.assertNotIn("bars", latest_run)
        self.assertNotIn("markers", latest_run)

    def test_dense_indicator_response_fits_transport_and_exposes_current_markers(self) -> None:
        run = dense_indicator_run()
        with mock.patch.object(self.server, "_request", return_value=[run]):
            result = self.server.get_indicator_results("indicator-id", "15m")
        text = indicator_text(result)
        self.assertLessEqual(len(text.encode("utf-8")), 24 * 1024)
        self.assertLessEqual(len(text.split("\n")), 1000)
        self.assertEqual(result[0]["markers"][0]["bar_index"], 499)

    def test_indicator_reads_reject_malformed_visual_data(self) -> None:
        run = dense_indicator_run(1, 1, 1)
        run["visual_data"]["markers"] = {}
        with mock.patch.object(
            self.server,
            "_request",
            return_value=[run],
        ):
            with self.assertRaisesRegex(RuntimeError, "visual data"):
                self.server.get_indicator_results("indicator-id", "1h")

    def test_indicator_reads_page_long_history_without_hiding_signals(self) -> None:
        run = dense_indicator_run()

        with mock.patch.object(self.server, "_request", return_value=[run]):
            result = self.server.get_indicator_results("indicator-id", "1h", run_id="frozen-run-id")[0]
        self.assertNotIn("candle_data", result)
        self.assertNotIn("plot_data", result)
        self.assertEqual(result["bar_count"], 500)
        self.assertEqual(result["bar_start"], 480)
        self.assertEqual(result["previous_bar_start"], 460)
        self.assertIsNone(result["next_bar_start"])
        self.assertEqual(len(result["bars"]), 20)
        self.assertEqual(result["bars"][0]["bar_index"], 480)
        self.assertEqual(result["bars"][-1]["plots"], run["latest_values"])
        self.assertEqual(result["markers"][0]["opened_at"], source_open(499))
        self.assertEqual(result["markers"][0]["closed_at"], source_open(500))
        self.assertFalse(result["markers_complete"])
        self.assertEqual(result["marker_count"], 2000)
        self.assertEqual(result["marker_returned_count"], 20)

        with mock.patch.object(self.server, "_request", return_value=[indicator_definition(run)]):
            latest = self.server.list_indicators()["items"][0]["latest_run"]
        self.assertEqual(latest["id"], result["id"])
        self.assertNotIn("bars", latest)
        self.assertNotIn("markers", latest)

        with mock.patch.object(self.server, "_request", return_value=[run]) as request:
            older = self.server.get_indicator_results("indicator-id", "1h", run_id="frozen-run-id", bar_start=0)[0]
        self.assertEqual(older["bar_start"], 0)
        self.assertIsNone(older["previous_bar_start"])
        self.assertEqual(older["next_bar_start"], 20)
        self.assertEqual(older["bars"][0]["plots"], {f"Plot {n}": 0 for n in range(3)})
        self.assertEqual(older["bars"][-1]["bar_index"], 19)
        self.assertEqual(request.call_args.kwargs["params"]["limit"], 1)

        with mock.patch.object(self.server, "_request", return_value=[run]):
            middle = self.server.get_indicator_results("indicator-id", "1h", run_id="frozen-run-id", bar_start=200, bar_limit=50)[0]
        count = middle["bar_returned_count"]
        self.assertGreater(count, 0)
        self.assertLessEqual(count, 50)
        self.assertEqual((middle["bar_start"], middle["previous_bar_start"], middle["next_bar_start"]), (200, 200 - count, 200 + count))
        self.assertEqual([bar["bar_index"] for bar in middle["bars"]], list(range(200, 200 + count)))
        self.assertEqual(middle["bars"][0]["closed_at"], source_open(201))
        self.assertEqual(middle["bars"][0]["high"], "201")
        self.assert_indicator_budget([middle])

    def assert_indicator_budget(self, result: Any) -> None:
        text = indicator_text(result)
        self.assertLessEqual(len(text.encode("utf-8")), self.server.INDICATOR_TEXT_BYTES)
        self.assertLessEqual(len(text.split("\n")), self.server.INDICATOR_TEXT_LINES)

    def read_indicator_page(self, run: dict[str, Any], **kwargs: Any) -> dict[str, Any]:
        with mock.patch.object(self.server, "_request", return_value=[run]):
            result = self.server.get_indicator_results("indicator-id", "15m", run_id=run["id"], **kwargs)
        self.assert_indicator_budget(result)
        return result[0]

    def test_maximum_density_and_aggregate_pages_fit_both_budgets(self) -> None:
        run = dense_indicator_run(2000, 32, 32)
        for marker in run["visual_data"]["markers"]:
            marker["title"] = "候" * 42 + "!!"  # 128 bytes.
        for args in ({}, {"bar_limit": 100, "marker_limit": 100}, {"bar_start": 0, "bar_limit": 100}):
            page = self.read_indicator_page(run, **args)
            self.assertGreater(page["bar_returned_count"], 0)
            self.assertGreater(page["marker_returned_count"], 0)
            self.assertFalse(page["bars_complete"])
            self.assertFalse(page["markers_complete"])
            self.assertIsNotNone(page["next_marker_start"])
            if "bar_start" in args:
                self.assertEqual(page["bars"][0]["bar_index"], 0)
                self.assertEqual(page["next_bar_start"], page["bar_returned_count"])
            else:
                self.assertEqual(page["bars"][-1]["bar_index"], 1999)
        with mock.patch.object(self.server, "_request", return_value=[run, run]):
            pages = self.server.get_indicator_results("indicator-id", "15m", limit=2, bar_limit=100, marker_limit=100)
        self.assertEqual(len(pages), 2)
        self.assert_indicator_budget(pages)
        self.assertTrue(any(page["budget_reduced"] for page in pages))

    def test_all_numeric_history_recovers_forward_and_backward_without_gaps(self) -> None:
        run = dense_indicator_run(500, 32, 4)
        for direction in ("forward", "backward"):
            collected = []
            args = {"bar_start": 0} if direction == "forward" else {}
            for iteration in range(501):
                # Vary marker/number requests to force changing fitted page sizes.
                page = self.read_indicator_page(run, bar_limit=100, marker_limit=100 if iteration % 2 else 1, **args)
                self.assertGreater(page["bar_returned_count"], 0)
                indices = [bar["bar_index"] for bar in page["bars"]]
                collected.extend(indices if direction == "forward" else reversed(indices))
                cursor = page["next_bar_start"] if direction == "forward" else page["previous_bar_end"]
                if cursor is None:
                    break
                args = {"bar_start" if direction == "forward" else "bar_end": cursor}
            else:
                self.fail("numeric pagination never finished")
            self.assertEqual(collected, list(range(500)) if direction == "forward" else list(reversed(range(500))))

    def test_all_marker_history_recovers_independently_with_stable_duplicate_order(self) -> None:
        run = dense_indicator_run()
        cursor = 0
        collected = []
        for _ in range(2001):
            page = self.read_indicator_page(run, marker_start=cursor, marker_limit=100, bar_limit=1)
            self.assertEqual(page["bars"][0]["bar_index"], 499)
            self.assertGreater(page["marker_returned_count"], 0)
            for event in page["markers"]:
                original = run["visual_data"]["markers"][event["event_position"]]
                self.assertEqual(event["opened_at"], source_open(original['bar_index']))
                self.assertEqual(event["closed_at"], source_open(original['bar_index'] + 1))
                for field in ("kind", "title", "text", "character", "value", "offset"):
                    self.assertEqual(event.get(field), original.get(field))
                collected.append(event["event_position"])
            next_cursor = page["next_marker_start"]
            if next_cursor is None:
                break
            self.assertEqual(next_cursor, cursor + page["marker_returned_count"])
            cursor = next_cursor
        else:
            self.fail("marker pagination never finished")
        expected = sorted(range(2000), key=lambda n: -run["visual_data"]["markers"][n]["bar_index"])
        self.assertEqual(collected, expected)
        empty = self.read_indicator_page(run, marker_start=2000)
        self.assertEqual(empty["markers"], [])
        self.assertFalse(empty["markers_complete"])
        self.assertLess(empty["previous_marker_start"], empty["marker_start"])
        previous = self.read_indicator_page(run, marker_start=empty["previous_marker_start"])
        self.assertGreater(previous["marker_returned_count"], 0)

    def test_discovery_recovers_catalog_without_embedding_histories(self) -> None:
        run = dense_indicator_run()
        catalog = [{"id": str(index), "name": "候" * 150, "enabled": True, "timeframes": ["5m", "15m"],
                    "active_version_id": "version-id", "latest_run": run, "source": "x" * 100_000,
                    "description": "x" * 100_000} for index in range(65)]
        for total in (2, 65):
            cursor, found = 0, []
            with mock.patch.object(self.server, "_request", return_value=catalog[:total]):
                for _ in range(total + 1):
                    page = self.server.list_indicators(limit=100, offset=cursor)
                    self.assert_indicator_budget(page)
                    self.assertEqual(page["total"], total)
                    self.assertEqual(page["returned_count"], len(page["items"]))
                    for item in page["items"]:
                        self.assertNotIn("source", item)
                        self.assertNotIn("description", item)
                        self.assertNotIn("bars", item["latest_run"])
                        self.assertNotIn("markers", item["latest_run"])
                        found.append(item["id"])
                    if page["next_offset"] is None:
                        break
                    self.assertGreater(page["next_offset"], cursor)
                    cursor = page["next_offset"]
            self.assertEqual(found, [str(index) for index in range(total)])

    def test_empty_missing_failed_and_skipped_evidence_are_distinct(self) -> None:
        empty = self.read_indicator_page(dense_indicator_run(0, 0, 0))
        self.assertTrue(empty["evidence_available"])
        self.assertTrue(empty["markers_complete"])
        self.assertTrue(empty["bars_complete"])
        self.assertEqual(empty["marker_count"], 0)
        for status in ("failed", "skipped", "queued", "succeeded"):
            run = dense_indicator_run()
            run.update(status=status, candle_data=None, plot_data=None, visual_data=None,
                       error_summary="錯" * 10000, diagnostics=[{"severity": "error", "message": "bad" * 1000}] * 100)
            page = self.read_indicator_page(run)
            self.assertEqual(page["status"], status)
            self.assertFalse(page["evidence_available"])
            self.assertFalse(page["markers_complete"])
            self.assertIsNone(page["marker_count"])
            self.assertTrue(page["diagnostics_abbreviated"])
            self.assertTrue(page["error_summary_abbreviated"])
        with mock.patch.object(self.server, "_request", return_value=[]):
            self.assertEqual(self.server.get_indicator_results("indicator-id", "15m"), [])

    def test_malformed_markers_candles_and_plots_return_bounded_errors(self) -> None:
        for change in ({"bar_index": -1}, {"bar_index": True}, {"bar_index": 2}, {"kind": "unknown"},
                       {"value": float("nan")}, {"offset": 1.5}, {"text": {}}, {"title": None}):
            run = dense_indicator_run(2, 1, 1)
            run["visual_data"]["markers"][0].update(change)
            with self.subTest(change=change), self.assertRaisesRegex(RuntimeError, "marker"):
                self.read_indicator_page(run)
        run = dense_indicator_run(2, 1, 1)
        run["candle_data"][0] = None
        with self.assertRaisesRegex(RuntimeError, "candle"):
            self.read_indicator_page(run)
        run = dense_indicator_run(2, 1, 1)
        run["plot_data"]["Plot 0"] = [1]
        with self.assertRaisesRegex(RuntimeError, "unaligned"):
            self.read_indicator_page(run)

    def test_oversized_single_units_and_multi_run_minimum_fail_explicitly(self) -> None:
        for target in ("title", "plot", "header"):
            run = dense_indicator_run(1, 1, 1)
            if target == "title":
                run["visual_data"]["markers"][0]["title"] = "x" * 30000
            elif target == "plot":
                run["plot_data"] = {"x" * 30000: [1]}
            else:
                run["latest_values"] = {"x" * 30000: 1}
            with self.assertRaisesRegex(ValueError, "oversized"):
                self.read_indicator_page(run)
        run = dense_indicator_run(1, 32, 1)
        with mock.patch.object(self.server, "_request", return_value=[run] * 100):
            with self.assertRaisesRegex(ValueError, "smaller run limit or exact run_id"):
                self.server.get_indicator_results("indicator-id", "15m", limit=100)
        run = dense_indicator_run(500, 32, 4)
        with mock.patch.object(self.server, "_request", return_value=[run] * 100):
            with self.assertRaisesRegex(ValueError, "smaller run limit"):
                self.server.get_indicator_results("indicator-id", "15m", limit=100, bar_limit=100, marker_limit=100)
        with mock.patch.object(self.server, "_request", return_value=[indicator_definition(run, name="x" * 30000)]):
            with self.assertRaisesRegex(ValueError, "discovery item"):
                self.server.list_indicators()

    def test_offsets_require_exact_runs_and_validate_ranges(self) -> None:
        for args in ({"bar_start": 0}, {"marker_start": 0}, {"bar_end": 20}):
            with self.assertRaisesRegex(ValueError, "exact run_id"):
                self.server.get_indicator_results("indicator-id", "15m", **args)
        for args in ({"marker_start": -1}, {"marker_start": True}, {"marker_limit": 0},
                     {"marker_limit": 101}, {"marker_limit": True}, {"bar_end": -1},
                     {"bar_start": 0, "bar_end": 1}, {"limit": 101}, {"limit": True}):
            with self.assertRaises(ValueError):
                self.read_indicator_page(dense_indicator_run(), **args)
        for args in ({"bar_start": 501}, {"bar_end": 501}, {"marker_start": 2001}):
            with self.assertRaisesRegex(ValueError, "exceeds"):
                self.read_indicator_page(dense_indicator_run(), **args)
        for args in ({"offset": -1}, {"offset": True}, {"limit": 0}, {"limit": 101}):
            with self.assertRaises(ValueError):
                self.server.list_indicators(**args)

    def test_continuations_forward_only_the_exact_authorized_run(self) -> None:
        run = dense_indicator_run()
        first = self.read_indicator_page(run)
        with mock.patch.object(self.server, "_request", return_value=[run]) as request:
            page = self.server.get_indicator_results("indicator-id", "15m", instrument_id="ETH",
                                                     run_id=first["id"], marker_start=first["next_marker_start"])[0]
        request.assert_called_once_with("GET", "/api/v1/indicators/indicator-id/results",
                                       params={"timeframe": "15m", "instrument_id": "ETH", "run_id": first["id"], "limit": 1})
        for field in ("id", "indicator_version_id", "instrument_id", "timeframe", "scheduled_for"):
            self.assertEqual(page[field], first[field])

    def test_output_diagnostics_log_counts_without_evidence_content(self) -> None:
        run = dense_indicator_run(500, 32, 4)
        with self.assertLogs(self.server.LOGGER, level="INFO") as logs:
            page = self.read_indicator_page(run, bar_limit=100, marker_limit=100)
        self.assertIn("tool=get_indicator_results schema_version=2", logs.output[0])
        self.assertIn(f"bars={page['bar_returned_count']} markers={page['marker_returned_count']}", logs.output[0])
        self.assertIn("budget_reduced=True", logs.output[0])
        self.assertNotIn("候補", logs.output[0])
        self.assertNotIn("frozen-run-id", logs.output[0])

    @unittest.skipUnless(REAL_MCP_AVAILABLE, "install mcp==1.28.1 for real transport checks")
    def test_real_fastmcp_serialization(self) -> None:
        completed = subprocess.run([sys.executable, str(MCP_DIR / "test_indicator_transport.py")],
                                   capture_output=True, text=True, timeout=120)
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)

    def test_indicator_result_bar_window_rejects_invalid_bounds(self) -> None:
        for start, size in [(-1, 100), (0, 0), (0, 101), (True, 10), (0, True)]:
            with self.subTest(start=start, size=size):
                with self.assertRaises(ValueError):
                    self.server.get_indicator_results("indicator-id", "1h", bar_start=start, bar_limit=size)

    def test_indicator_mutations_use_plural_timeframes(self) -> None:
        with mock.patch.object(self.server, "_request", return_value={"id": "indicator-id"}) as request:
            self.server.create_indicator(
                "EMA",
                ["15m", "1h"],
                ["BTC"],
                'indicator("EMA")',
            )
        self.assertEqual(
            request.call_args.kwargs["json_body"]["timeframes"],
            ["15m", "1h"],
        )
        self.assertNotIn("timeframe", request.call_args.kwargs["json_body"])

        with self.assertRaisesRegex(ValueError, "duplicates"):
            self.server.create_indicator(
                "EMA",
                ["1h", " 1h "],
                ["BTC"],
                'indicator("EMA")',
            )

    def test_indicator_results_require_a_timeframe(self) -> None:
        with mock.patch.object(self.server, "_request", return_value=[]) as request:
            self.server.get_indicator_results("indicator-id", "4h", instrument_id="BTC")
        self.assertEqual(
            request.call_args.kwargs["params"],
            {"timeframe": "4h", "instrument_id": "BTC", "limit": 1},
        )

    def test_indicator_results_accept_an_exact_run_id(self) -> None:
        with mock.patch.object(self.server, "_request", return_value=[]) as request:
            self.server.get_indicator_results("indicator-id", "1h", run_id="run-id")
        self.assertEqual(
            request.call_args.kwargs["params"],
            {"timeframe": "1h", "run_id": "run-id", "limit": 1},
        )

    def test_strategy_prompt_tools_use_authenticated_api_paths(self) -> None:
        captured: list[dict[str, object]] = []

        def fake_request(method, path, *, params=None, json_body=None):
            captured.append(
                {
                    "method": method,
                    "path": path,
                    "params": params,
                    "json_body": json_body,
                }
            )
            response = {
                "revision_id": 1,
                "target_sub_agent_id": 7,
                "target_sub_agent_key": "technical-1h",
                "prompt": "Review market structure.",
                "updated_at": "2026-08-20T00:00:00Z",
            }
            return [response] if method == "GET" and path.endswith("prompts") else response

        with mock.patch.object(self.server, "_request", side_effect=fake_request):
            listed = self.server.list_strategy_prompts()
            fetched = self.server.get_strategy_prompt(7)
            updated = self.server.update_strategy_prompt(7, "  Keep it concise.  ")

        self.assertEqual(len(listed), 1)
        self.assertEqual(fetched["target_sub_agent_key"], "technical-1h")
        self.assertEqual(updated["prompt"], "Review market structure.")
        self.assertEqual(
            captured,
            [
                {
                    "method": "GET",
                    "path": "/api/v1/strategy-prompts",
                    "params": None,
                    "json_body": None,
                },
                {
                    "method": "GET",
                    "path": "/api/v1/strategy-prompts/7",
                    "params": None,
                    "json_body": None,
                },
                {
                    "method": "PUT",
                    "path": "/api/v1/strategy-prompts/7",
                    "params": None,
                    "json_body": {"prompt": "  Keep it concise.  "},
                },
            ],
        )

    def test_list_analysis_instruments_uses_the_discovery_endpoint(self) -> None:
        with mock.patch.object(self.server, "_request", return_value=["BTC", "ETH"]) as request:
            result = self.server.list_analysis_instruments()

        self.assertEqual(result, ["BTC", "ETH"])
        request.assert_called_once_with("GET", "/api/v1/analysis-instruments")

    def test_trading_instrument_tools_use_the_authenticated_api_paths(self) -> None:
        with mock.patch.object(self.server, "_request", return_value=["BTC", "ETH"]) as request:
            listed = self.server.list_trading_instruments()
            updated = self.server.set_trading_instrument_enabled("ETH", True)

        self.assertEqual(listed, ["BTC", "ETH"])
        self.assertEqual(updated, ["BTC", "ETH"])
        self.assertEqual(
            request.call_args_list,
            [
                mock.call("GET", "/api/v1/trading-instruments"),
                mock.call(
                    "PUT",
                    "/api/v1/trading-instruments/ETH",
                    json_body={"enabled": True},
                ),
            ],
        )

    def test_strategy_prompt_tools_validate_inputs_and_response_shape(self) -> None:
        with self.assertRaises(ValueError):
            self.server.get_strategy_prompt(0)
        with self.assertRaises(ValueError):
            self.server.update_strategy_prompt(7, None)  # type: ignore[arg-type]
        with mock.patch.object(self.server, "_request", return_value={"prompt": "missing"}):
            with self.assertRaisesRegex(RuntimeError, "unexpected shape"):
                self.server.get_strategy_prompt(7)

    def test_submit_prompt_revision_uses_canonical_change_fields(self) -> None:
        captured: dict[str, object] = {}

        def fake_request(method, path, *, params=None, json_body=None):
            captured.update(method=method, path=path, json_body=json_body)
            return {"batch_id": 12}

        with mock.patch.object(self.server, "_request", side_effect=fake_request):
            result = self.server.submit_prompt_revision(
                rationale="Clarify the fill-protection workflow.",
                evidence_memory_ids=["00000000-0000-0000-0000-000000000001"],
                changes=[
                    {
                        "target_sub_agent_id": 2,
                        "base_revision_id": 3,
                        "prompt": "Updated prompt.",
                    }
                ],
            )

        self.assertEqual(result, {"batch_id": 12})
        self.assertEqual(captured["method"], "POST")
        self.assertEqual(captured["path"], "/api/v1/strategy-prompts/revisions")
        self.assertEqual(
            captured["json_body"],
            {
                "rationale": "Clarify the fill-protection workflow.",
                "evidence_memory_ids": ["00000000-0000-0000-0000-000000000001"],
                "changes": [
                    {
                        "target_sub_agent_id": 2,
                        "base_revision_id": 3,
                        "prompt": "Updated prompt.",
                    }
                ],
            },
        )

    def test_logger_has_a_stderr_handler(self) -> None:
        self.assertTrue(
            any(
                isinstance(handler, self.server.logging.StreamHandler)
                for handler in self.server.LOGGER.handlers
            )
        )

    def test_container_log_redirection_is_best_effort(self) -> None:
        with mock.patch.object(self.server.os, "open", side_effect=OSError):
            self.server._redirect_stderr_to_container_log()

    def test_main_treats_broken_stdio_as_a_clean_shutdown(self) -> None:
        broken_resource = type("BrokenResourceError", (Exception,), {})()
        error = ExceptionGroup("stdio closed", [broken_resource])
        with (
            mock.patch.object(self.server, "_redirect_stderr_to_container_log"),
            mock.patch.object(self.server.mcp, "run", side_effect=error),
        ):
            self.server.main()

    def test_missing_env_raises_with_clear_message(self) -> None:
        with mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaises(RuntimeError) as ctx:
                self.server._load_config()
        message = str(ctx.exception)
        self.assertIn("HYPERVIBES_API_BASE_URL", message)
        self.assertIn("HYPERVIBES_API_KEY", message)
        self.assertIn("HYPERVIBES_AGENT_KEY", message)

    def test_headers_use_bearer_token(self) -> None:
        with mock.patch.dict(
            os.environ,
            {
                "HYPERVIBES_API_BASE_URL": "http://example.test",
                "HYPERVIBES_API_KEY": "vta_secret",
                "HYPERVIBES_AGENT_KEY": "btc-2",
            },
            clear=True,
        ):
            setattr(self.server, "CONFIG", ("", "", ""))
            headers = self.server._headers()
        self.assertEqual(headers, {"Authorization": "Bearer vta_secret"})

    def test_get_trading_context_query_construction(self) -> None:
        captured: dict[str, object] = {}

        def fake_request(method, path, *, params=None, json_body=None):
            captured["method"] = method
            captured["path"] = path
            captured["params"] = params
            captured["json_body"] = json_body
            return {"instrument_id": "BTC", "evidence": []}

        with mock.patch.dict(
            os.environ,
            {
                "HYPERVIBES_API_BASE_URL": "http://example.test",
                "HYPERVIBES_API_KEY": "k",
                "HYPERVIBES_AGENT_KEY": "a",
            },
            clear=True,
        ):
            setattr(self.server, "CONFIG", self.server._load_config())
            with mock.patch.object(self.server, "_request", side_effect=fake_request):
                self.server.get_trading_context("BTC")
        self.assertEqual(captured["method"], "GET")
        self.assertEqual(captured["path"], "/api/v1/memories/trading-context")
        self.assertEqual(
            captured["params"],
            {"instrument_id": "BTC"},
        )

    def test_trading_context_rejects_blank_instrument_id(self) -> None:
        with self.assertRaises(ValueError):
            self.server.get_trading_context("   ")

    def test_request_logs_safe_response_shape(self) -> None:
        response = mock.Mock()
        response.status_code = 200
        response.content = b"[]"
        response.json.return_value = []
        with self.assertLogs(self.server.LOGGER, level="INFO") as logs:
            with mock.patch.object(self.server.httpx, "request", return_value=response):
                result = self.server._request(
                    "GET",
                    "/api/v1/memories",
                    params={"instrument_id": "BTC"},
                )
        self.assertEqual(result, [])
        self.assertIn("path=/api/v1/memories", logs.output[0])
        self.assertIn("response=list:0", logs.output[0])

    def test_list_memories_builds_agent_scoped_review_query(self) -> None:
        captured: dict[str, object] = {}

        def fake_request(method, path, *, params=None, json_body=None):
            captured["method"] = method
            captured["path"] = path
            captured["params"] = params
            return []

        with mock.patch.object(self.server, "_request", side_effect=fake_request):
            self.server.list_memories(
                scope_kind="agent",
                memory_type="review",
                include_expired=True,
                limit=1,
            )
        self.assertEqual(captured["method"], "GET")
        self.assertEqual(captured["path"], "/api/v1/memories")
        self.assertEqual(
            captured["params"],
            {
                "scope_kind": "agent",
                "memory_type": "review",
                "include_expired": "true",
                "limit": 1,
            },
        )

    def test_list_account_transactions_query_construction(self) -> None:
        captured: dict[str, object] = {}

        def fake_request(method, path, *, params=None, json_body=None):
            captured["method"] = method
            captured["path"] = path
            captured["params"] = params
            return []

        with mock.patch.object(self.server, "_request", side_effect=fake_request):
            self.server.list_account_transactions(
                "2026-07-16T00:00:00Z",
                "2026-07-17T00:00:00Z",
                symbol="BTC",
                event_category="fill",
                limit=100,
                offset=200,
            )
        self.assertEqual(captured["method"], "GET")
        self.assertEqual(captured["path"], "/api/v1/account/transactions")
        self.assertEqual(
            captured["params"],
            {
                "since": "2026-07-16T00:00:00Z",
                "until": "2026-07-17T00:00:00Z",
                "symbol": "BTC",
                "event_category": "fill",
                "limit": 100,
                "offset": 200,
            },
        )

    def test_list_account_transactions_rejects_negative_offset(self) -> None:
        with self.assertRaises(ValueError):
            self.server.list_account_transactions(
                "2026-07-16T00:00:00Z",
                "2026-07-17T00:00:00Z",
                offset=-1,
            )

    def test_trade_and_note_tools_construct_scoped_requests(self) -> None:
        calls: list[tuple[object, ...]] = []

        def fake_request(method, path, *, params=None, json_body=None):
            calls.append((method, path, params, json_body))
            return {} if json_body is not None or "/trades/" in path else []

        trade_id = "00000000-0000-0000-0000-000000000001"
        with mock.patch.object(self.server, "_request", side_effect=fake_request):
            self.server.list_account_trades(limit=10, offset=20)
            self.server.get_account_trade(trade_id)
            self.server.list_journal_notes("fill", "hash:12")
            self.server.add_journal_note("trade", trade_id, "  journal entry  ")
        self.assertEqual(calls[0], ("GET", "/api/v1/account/trades", {"limit": 10, "offset": 20}, None))
        self.assertEqual(calls[1][1], f"/api/v1/account/trades/{trade_id}")
        self.assertEqual(calls[2][1], "/api/v1/account/journal/fill/hash%3A12/notes")
        self.assertEqual(calls[3], ("POST", f"/api/v1/account/journal/trade/{trade_id}/notes", None,
                                    {"body": "journal entry"}))

    def test_journal_note_tools_reject_invalid_inputs(self) -> None:
        with self.assertRaises(ValueError):
            self.server.list_account_trades(offset=-1)
        with self.assertRaises(ValueError):
            self.server.get_account_trade("not-a-uuid")
        with self.assertRaises(ValueError):
            self.server.add_journal_note("ledger", "a/b", "note")
        with self.assertRaises(ValueError):
            self.server.add_journal_note("ledger", "id", " " * 4)

    def test_write_memory_defaults_metadata_to_empty_object(self) -> None:
        captured: dict[str, object] = {}

        def fake_request(method, path, *, params=None, json_body=None):
            captured["json_body"] = json_body
            return {
                "id": "00000000-0000-0000-0000-000000000000",
                "summary": "ok",
            }

        with mock.patch.dict(
            os.environ,
            {
                "HYPERVIBES_API_BASE_URL": "http://example.test",
                "HYPERVIBES_API_KEY": "k",
                "HYPERVIBES_AGENT_KEY": "a",
            },
            clear=True,
        ):
            setattr(self.server, "CONFIG", self.server._load_config())
            with mock.patch.object(self.server, "_request", side_effect=fake_request):
                self.server.write_memory(
                    scope_kind="instruments",
                    instrument_ids=["BTC"],
                    memory_type="trend_analysis",
                    summary="summary",
                    content="content",
                )
        self.assertEqual(
            captured["json_body"],
            {
                "scope_kind": "instruments",
                "instrument_ids": ["BTC"],
                "memory_type": "trend_analysis",
                "summary": "summary",
                "content": "content",
                "metadata": {},
            },
        )

    def test_write_memory_rejects_non_object_metadata(self) -> None:
        with self.assertRaises(ValueError):
            self.server.write_memory(
                scope_kind="agent",
                memory_type="analysis",
                summary="s",
                content="c",
                metadata=["not", "a", "dict"],
            )

    def test_write_memory_validates_scope_and_targets(self) -> None:
        with self.assertRaises(ValueError):
            self.server.write_memory(
                scope_kind="agent",
                instrument_ids=["BTC"],
                memory_type="review",
                summary="summary",
                content="content",
            )
        with self.assertRaises(ValueError):
            self.server.write_memory(
                scope_kind="instruments",
                instrument_ids=["BTC", "BTC"],
                memory_type="trend_analysis",
                summary="summary",
                content="content",
            )
        with self.assertRaises(ValueError):
            self.server.write_memory(
                scope_kind="instruments",
                memory_type="trend_analysis",
                summary="summary",
                content="content",
            )

    def test_write_memory_rejects_spoofed_run_provenance(self) -> None:
        with self.assertRaisesRegex(ValueError, "provenance"):
            self.server.write_memory(
                scope_kind="agent",
                memory_type="review",
                summary="summary",
                content="content",
                metadata={"source_run_id": 42},
            )

    def test_submit_orders_requires_non_empty_list(self) -> None:
        with self.assertRaises(ValueError):
            self.server.submit_orders([])
        with self.assertRaises(ValueError):
            self.server.submit_orders("not-a-list")  # type: ignore[arg-type]

    def test_order_schemas_reject_run_214_aliases_and_missing_limit_price(self) -> None:
        with self.assertRaises(ValidationError):
            self.server.CancelOrderInput.model_validate(
                {"symbol": "ETH", "oid": "549220142898"}
            )
        with self.assertRaises(ValidationError):
            self.server.CancelOrderInput.model_validate(
                {"coin": "ETH", "oid": 549220142898}
            )
        with self.assertRaises(ValidationError):
            self.server.SubmitOrderInput.model_validate(
                {
                    "symbol": "ETH",
                    "side": "buy",
                    "order_type": "limit",
                    "sz": 0.004,
                    "limit_px": 2606,
                }
            )
        with self.assertRaisesRegex(ValidationError, "price is required"):
            self.server.SubmitOrderInput.model_validate(
                {
                    "symbol": "ETH",
                    "side": "buy",
                    "order_type": "limit",
                    "size": 0.004,
                }
            )
        with self.assertRaises(ValidationError):
            self.server.SubmitOrderInput.model_validate(
                {
                    "symbol": "ETH",
                    "side": "buy",
                    "order_type": "limit",
                    "size": 0.004,
                    "price": 2606,
                    "limit_px": 2606,
                }
            )
        with self.assertRaises(ValidationError):
            self.server.SubmitOrderInput.model_validate(
                {
                    "symbol": "ETH",
                    "side": "buy",
                    "order_type": "limit",
                    "size": 0.004,
                    "price": 2606,
                    "tif": "Gtc",
                }
            )

    def test_order_schemas_expose_only_the_backend_contract(self) -> None:
        submit_schema = self.server.SubmitOrderInput.model_json_schema()
        self.assertFalse(submit_schema["additionalProperties"])
        self.assertEqual(
            set(submit_schema["required"]),
            {"symbol", "side", "order_type", "size"},
        )
        self.assertTrue(
            {
                "symbol",
                "side",
                "order_type",
                "size",
                "price",
                "time_in_force",
                "reduce_only",
                "take_profits",
                "stop_losses",
                "memory_record_ids",
                "attribution_source",
            }.issubset(submit_schema["properties"])
        )

        cancel_schema = self.server.CancelOrderInput.model_json_schema()
        self.assertFalse(cancel_schema["additionalProperties"])
        self.assertEqual(set(cancel_schema["required"]), {"symbol", "oid"})
        self.assertEqual(cancel_schema["properties"]["oid"]["type"], "integer")

    def test_order_tools_serialize_validated_payloads(self) -> None:
        captured: dict[str, Any] = {}

        def fake_request(method, path, *, params=None, json_body=None):
            captured.update(method=method, path=path, json_body=json_body)
            return {"results": []} if path == "/api/v1/orders" else []

        order = self.server.SubmitOrderInput.model_validate(
            {
                "symbol": "ETH",
                "side": "buy",
                "order_type": "limit",
                "size": 0.004,
                "price": 2606,
                "time_in_force": "gtc",
                "memory_record_ids": ["decision-id"],
            }
        )
        cancel = self.server.CancelOrderInput.model_validate(
            {"symbol": "ETH", "oid": 549220142898}
        )

        with mock.patch.object(self.server, "_request", side_effect=fake_request):
            self.server.submit_orders([order])
        self.assertEqual(captured["method"], "POST")
        self.assertEqual(captured["path"], "/api/v1/orders")
        self.assertEqual(
            captured["json_body"],
            {
                "orders": [
                    {
                        "symbol": "ETH",
                        "side": "buy",
                        "order_type": "limit",
                        "size": "0.004",
                        "price": "2606",
                        "time_in_force": "gtc",
                        "reduce_only": False,
                        "take_profits": [],
                        "stop_losses": [],
                        "memory_record_ids": ["decision-id"],
                        "attribution_source": "agent",
                    }
                ]
            },
        )

        with mock.patch.object(self.server, "_request", side_effect=fake_request):
            self.server.cancel_orders([cancel])
        self.assertEqual(captured["path"], "/api/v1/orders/cancel")
        self.assertEqual(
            captured["json_body"],
            {"orders": [{"symbol": "ETH", "oid": 549220142898}]},
        )

    def test_memory_detail_requests_links(self) -> None:
        captured: dict[str, object] = {}

        def fake_request(method, path, **kwargs):
            captured.update(method=method, path=path, **kwargs)
            return {"id": "memory", "links_from": [], "links_to": []}

        with mock.patch.object(self.server, "_request", side_effect=fake_request):
            result = self.server.get_memory_detail("memory-id")
        self.assertEqual(result["id"], "memory")
        self.assertEqual(captured["path"], "/api/v1/memories/memory-id")
        self.assertEqual(captured["params"], {"include": "links"})

    def test_error_message_redacts_api_key(self) -> None:
        with mock.patch.dict(
            os.environ,
            {
                "HYPERVIBES_API_BASE_URL": "http://example.test",
                "HYPERVIBES_API_KEY": "vta_super_secret",
                "HYPERVIBES_AGENT_KEY": "a",
            },
            clear=True,
        ):
            setattr(self.server, "CONFIG", self.server._load_config())

            class FakeResponse:
                status_code = 500
                text = "boom vta_super_secret boom"
                content = b"{}"

                def json(self):
                    return {}

            with mock.patch.object(self.server.httpx, "request", return_value=FakeResponse()):
                with self.assertRaises(RuntimeError) as ctx:
                    self.server._request("GET", "/api/v1/account")
        self.assertNotIn("vta_super_secret", str(ctx.exception))
        self.assertIn("[redacted]", str(ctx.exception))

    def test_memory_write_errors_preserve_recovery_code_without_automatic_retry(self) -> None:
        for status, body in [(422, {"error": "links[1].target_memory_id is unavailable; use same-agent context",
                                    "code": "invalid_memory_link", "link_index": 1}),
                             (500, {"error": "internal server error"})]:
            response = self.server.httpx.Response(status, json=body)
            with mock.patch.object(self.server, "_require_config", return_value=("http://example.test", "test-key", "agent")):
                with mock.patch.object(self.server.httpx, "request", return_value=response) as request:
                    with self.assertRaises(RuntimeError) as error:
                        self.server.write_memory("agent", "observation", "summary", "body")
            self.assertEqual(request.call_count, 1)
            self.assertIn(f"returned {status}:", str(error.exception))
            if status == 422:
                self.assertIn('"code":"invalid_memory_link"', str(error.exception))
                self.assertIn('"link_index":1', str(error.exception))

    def test_send_notification_posts_to_notifications_endpoint(self) -> None:
        captured: dict[str, Any] = {}

        def fake_request(method, path, *, params=None, json_body=None):
            captured["method"] = method
            captured["path"] = path
            captured["json_body"] = json_body
            return {"id": "00000000-0000-0000-0000-000000000000"}

        with mock.patch.object(self.server, "_request", side_effect=fake_request):
            result = self.server.send_notification(
                title="Position opened",
                body="BTC/USDC long 0.1",
                severity="info",
            )
        self.assertEqual(captured["method"], "POST")
        self.assertEqual(captured["path"], "/api/v1/notifications")
        self.assertEqual(
            captured["json_body"],
            {
                "title": "Position opened",
                "body": "BTC/USDC long 0.1",
                "severity": "info",
            },
        )
        self.assertEqual(result["id"], "00000000-0000-0000-0000-000000000000")

    def test_send_notification_defaults_severity_to_info(self) -> None:
        captured: dict[str, Any] = {}

        def fake_request(method, path, *, params=None, json_body=None):
            captured["json_body"] = json_body
            return {"id": "id"}

        with mock.patch.object(self.server, "_request", side_effect=fake_request):
            self.server.send_notification(title="t", body="b")
        self.assertEqual(captured["json_body"]["severity"], "info")

    def test_send_notification_rejects_blank_title(self) -> None:
        with self.assertRaises(ValueError):
            self.server.send_notification(title="  ", body="body")

    def test_send_notification_rejects_unknown_severity(self) -> None:
        with self.assertRaises(ValueError):
            self.server.send_notification(title="t", body="b", severity="critical")


class OhlcvProvenanceTests(unittest.TestCase):
    def test_saved_rows_keep_exact_values_and_open_close_boundary_provenance(self) -> None:
        path = REPO_ROOT / "agent-runtime/workspace-template/.opencode/skills/hyperliquid-data/fetch_ohlcv.py"
        spec = importlib.util.spec_from_file_location("ohlcv_helper", path)
        assert spec is not None and spec.loader is not None
        helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(helper)
        candles = [
            {"t": 1783112400000, "o": "2690.9", "h": "2691.6", "l": "2687.1", "c": "2688.8", "v": "12.5"},
            {"t": 1783113300000, "o": "2700", "h": "2701", "l": "2699", "c": "2700", "v": "10"},
        ]
        boundary = 1783113300000
        closed = helper.filter_closed_before(candles, 900000, boundary)
        payload = helper.canonical_payload("ETH", "15m", 900000, closed, boundary)
        self.assertEqual(payload["requested_boundary_ms"], boundary)
        self.assertEqual(len(payload["candles"]), 1)
        row = payload["candles"][0]
        self.assertEqual(row["timestamp_ms"], 1783112400000)
        self.assertEqual(row["opened_at"], "2026-07-03T21:00:00Z")
        self.assertEqual(row["closed_at"], "2026-07-03T21:15:00Z")
        self.assertEqual([row[key] for key in ("open", "high", "low", "close")], [2690.9, 2691.6, 2687.1, 2688.8])


if __name__ == "__main__":
    unittest.main()

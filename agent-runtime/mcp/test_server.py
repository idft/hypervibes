"""Lightweight tests for the HyperVibes MCP server helpers.

These tests intentionally avoid importing :mod:`mcp.server.fastmcp` so they
can run in environments where the MCP package is not installed (such as
the bare dev container). The MCP server itself is verified at runtime by
booting it inside the custom OpenCode image; these tests cover the parts
that can be exercised cheaply: env loading, validation, and the HTTP
helper's URL/header construction.
"""

from __future__ import annotations

import importlib
import importlib.util
import os
import subprocess
import sys
import tempfile
import types
import unittest
from pathlib import Path
from typing import Any
from unittest import mock

REPO_ROOT = Path(__file__).resolve().parents[2]
MCP_DIR = REPO_ROOT / "agent-runtime" / "mcp"


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

    def test_server_registers_the_full_tool_set(self) -> None:
        registered = set(self.server.mcp.tools)
        self.assertTrue(
            {
                "get_account",
                "list_strategy_prompts",
                "get_strategy_prompt",
                "update_strategy_prompt",
                "get_latest_analysis",
                "get_market_analysis",
                "get_memory_detail",
                "list_memories",
                "list_orders",
                "list_account_transactions",
                "get_order",
                "write_memory",
                "submit_orders",
                "cancel_orders",
                "cancel_all_orders",
                "coding_validate_candidate",
                "coding_submit_report",
            }.issubset(registered)
        )

    def test_automated_job_profiles_deny_strategy_prompt_tools(self) -> None:
        profiles = (
            Path(__file__).parents[2]
            / "agent-runtime"
            / "workspace-template"
            / ".opencode"
            / "agents"
        )
        for profile_name in [
            "analysis.md",
            "market-analysis.md",
            "trading.md",
            "daily-review.md",
            "analysis-coding.md",
        ]:
            profile = (profiles / profile_name).read_text(encoding="utf-8")
            self.assertIn("hypervibes_*: deny", profile, profile_name)
            self.assertNotIn("hypervibes_list_strategy_prompts:", profile, profile_name)
            self.assertNotIn("hypervibes_get_strategy_prompt:", profile, profile_name)
            self.assertNotIn("hypervibes_update_strategy_prompt:", profile, profile_name)

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
                "prompt_kind": "analysis",
                "prompt": "Review market structure.",
                "updated_at": "2026-08-20T00:00:00Z",
            }
            return [response] if method == "GET" and path.endswith("prompts") else response

        with mock.patch.object(self.server, "_request", side_effect=fake_request):
            listed = self.server.list_strategy_prompts()
            fetched = self.server.get_strategy_prompt("analysis")
            updated = self.server.update_strategy_prompt("analysis", "  Keep it concise.  ")

        self.assertEqual(len(listed), 1)
        self.assertEqual(fetched["prompt_kind"], "analysis")
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
                    "path": "/api/v1/strategy-prompts/analysis",
                    "params": None,
                    "json_body": None,
                },
                {
                    "method": "PUT",
                    "path": "/api/v1/strategy-prompts/analysis",
                    "params": None,
                    "json_body": {"prompt": "  Keep it concise.  "},
                },
            ],
        )

    def test_strategy_prompt_tools_validate_inputs_and_response_shape(self) -> None:
        with self.assertRaises(ValueError):
            self.server.get_strategy_prompt("invalid")
        with self.assertRaises(ValueError):
            self.server.update_strategy_prompt("analysis", None)  # type: ignore[arg-type]
        with mock.patch.object(self.server, "_request", return_value={"prompt": "missing"}):
            with self.assertRaisesRegex(RuntimeError, "unexpected shape"):
                self.server.get_strategy_prompt("analysis")

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

    def test_get_latest_analysis_query_construction(self) -> None:
        captured: dict[str, object] = {}

        def fake_request(method, path, *, params=None, json_body=None):
            captured["method"] = method
            captured["path"] = path
            captured["params"] = params
            captured["json_body"] = json_body
            return []

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
                self.server.get_latest_analysis("BTC", limit=3)
        self.assertEqual(captured["method"], "GET")
        self.assertEqual(captured["path"], "/api/v1/memories/latest")
        self.assertEqual(
            captured["params"],
            {"symbol": "BTC", "memory_type": "analysis", "limit": 3},
        )

    def test_validation_rejects_blank_symbol(self) -> None:
        with self.assertRaises(ValueError):
            self.server.get_latest_analysis("   ")

    def test_get_market_analysis_query_construction(self) -> None:
        captured: dict[str, object] = {}

        def fake_request(method, path, *, params=None, json_body=None):
            captured["method"] = method
            captured["path"] = path
            captured["params"] = params
            captured["json_body"] = json_body
            return []

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
                self.server.get_market_analysis("BTC")
        self.assertEqual(captured["method"], "GET")
        self.assertEqual(captured["path"], "/api/v1/memories")
        self.assertEqual(
            captured["params"],
            {"symbol": "BTC", "memory_type": "market_analysis", "limit": 1},
        )

    def test_get_market_analysis_rejects_blank_symbol(self) -> None:
        with self.assertRaises(ValueError):
            self.server.get_market_analysis("   ")

    def test_get_market_analysis_returns_none_for_no_rows(self) -> None:
        with self.assertLogs(self.server.LOGGER, level="INFO") as logs:
            with mock.patch.object(self.server, "_request", return_value=[]):
                self.assertIsNone(self.server.get_market_analysis("BTC"))
        self.assertIn(
            "hypervibes_mcp_market_analysis symbol=BTC found=false",
            logs.output[0],
        )

    def test_get_market_analysis_returns_first_row(self) -> None:
        row = {"symbol": "BTC", "memory_type": "market_analysis"}
        with self.assertLogs(self.server.LOGGER, level="INFO") as logs:
            with mock.patch.object(self.server, "_request", return_value=[row]):
                self.assertEqual(self.server.get_market_analysis("BTC"), row)
        self.assertIn(
            "hypervibes_mcp_market_analysis symbol=BTC found=true",
            logs.output[0],
        )

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
                    params={"symbol": "BTC", "memory_type": "market_analysis"},
                )
        self.assertEqual(result, [])
        self.assertIn("path=/api/v1/memories", logs.output[0])
        self.assertIn("response=list:0", logs.output[0])

    def test_validation_rejects_zero_limit(self) -> None:
        with self.assertRaises(ValueError):
            self.server.get_latest_analysis("BTC", limit=0)

    def test_list_memories_builds_historical_daily_review_query(self) -> None:
        captured: dict[str, object] = {}

        def fake_request(method, path, *, params=None, json_body=None):
            captured["method"] = method
            captured["path"] = path
            captured["params"] = params
            return []

        with mock.patch.object(self.server, "_request", side_effect=fake_request):
            self.server.list_memories(
                symbol="__agent__",
                memory_type="daily_review",
                include_expired=True,
                limit=1,
            )
        self.assertEqual(captured["method"], "GET")
        self.assertEqual(captured["path"], "/api/v1/memories")
        self.assertEqual(
            captured["params"],
            {
                "symbol": "__agent__",
                "memory_type": "daily_review",
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
                    symbol="BTC",
                    memory_type="analysis",
                    summary="summary",
                    content="content",
                )
        self.assertEqual(
            captured["json_body"],
            {
                "symbol": "BTC",
                "memory_type": "analysis",
                "summary": "summary",
                "content": "content",
                "metadata": {},
            },
        )

    def test_write_memory_rejects_non_object_metadata(self) -> None:
        with self.assertRaises(ValueError):
            self.server.write_memory(
                symbol="BTC",
                memory_type="analysis",
                summary="s",
                content="c",
                metadata=["not", "a", "dict"],
            )

    def test_write_memory_rejects_blank_timeframe(self) -> None:
        with self.assertRaises(ValueError):
            self.server.write_memory(
                symbol="BTC",
                memory_type="market_analysis",
                summary="summary",
                content="content",
                timeframe="   ",
            )

    def test_submit_orders_requires_non_empty_list(self) -> None:
        with self.assertRaises(ValueError):
            self.server.submit_orders([])
        with self.assertRaises(ValueError):
            self.server.submit_orders("not-a-list")  # type: ignore[arg-type]

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

    def test_coding_validation_runs_fixed_local_validator(self) -> None:
        coding = _load_server(
            {
                "HYPERVIBES_API_BASE_URL": "http://example.test",
                "HYPERVIBES_API_KEY": "k",
                "HYPERVIBES_AGENT_KEY": "a",
                "HYPERVIBES_CODING_TASK_ID": "42",
            }
        )
        with tempfile.TemporaryDirectory() as tmp:
            workspace = Path(tmp) / "workspace"
            (workspace / "scripts/user").mkdir(parents=True)
            (workspace / "scripts/user/analyze.py").write_text("print(1)")
            completed = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout='{"ok": true, "checks": ["compile", "contract"]}',
                stderr="",
            )
            prior = os.getcwd()
            os.chdir(workspace)
            try:
                with mock.patch.dict(
                    os.environ,
                    {"HYPERVIBES_CODING_TASK_ID": "42"},
                ):
                    with mock.patch.object(
                        coding.subprocess, "run", return_value=completed
                    ) as run:
                        result = coding.coding_validate_candidate()
                self.assertTrue(result["ok"])
                self.assertEqual(result["task_id"], 42)
                self.assertEqual(len(result["candidate_manifest_sha256"]), 64)
                self.assertTrue(
                    (workspace.parent / "coding-validation.json").is_file()
                )
                command = run.call_args.args[0]
                self.assertEqual(command[0], coding.CODING_VALIDATOR_PYTHON)
                self.assertEqual(command[1], coding.CODING_VALIDATOR_SCRIPT)
            finally:
                os.chdir(prior)

    def test_coding_manifest_rejects_unapproved_extension(self) -> None:
        coding = _load_server(
            {
                "HYPERVIBES_API_BASE_URL": "http://example.test",
                "HYPERVIBES_API_KEY": "k",
                "HYPERVIBES_AGENT_KEY": "a",
            }
        )
        with tempfile.TemporaryDirectory() as tmp:
            workspace = Path(tmp)
            user = workspace / "scripts/user"
            user.mkdir(parents=True)
            (user / "notes.txt").write_text("not approved")
            prior = os.getcwd()
            os.chdir(workspace)
            try:
                with self.assertRaisesRegex(RuntimeError, "extension is not allowed"):
                    coding._coding_manifest_hash()
            finally:
                os.chdir(prior)

    def test_coding_report_uses_task_id_from_environment(self) -> None:
        coding = _load_server(
            {
                "HYPERVIBES_API_BASE_URL": "http://example.test",
                "HYPERVIBES_API_KEY": "k",
                "HYPERVIBES_AGENT_KEY": "a",
                "HYPERVIBES_CODING_TASK_ID": "42",
            }
        )
        captured: dict[str, object] = {}

        def fake_request(method, path, **kwargs):
            captured.update(method=method, path=path, **kwargs)
            return {"submitted": True}

        with mock.patch.dict(os.environ, {"HYPERVIBES_CODING_TASK_ID": "42"}):
            with mock.patch.object(coding, "_request", side_effect=fake_request):
                result = coding.coding_submit_report(
                    "no_change", "none", "no evidence", [], [], "tests passed"
                )
        self.assertEqual(result, {"submitted": True})
        self.assertEqual(captured["path"], "/api/v1/coding/report")
        self.assertEqual(captured["json_body"]["task_id"], 42)  # type: ignore[index]

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


if __name__ == "__main__":
    unittest.main()

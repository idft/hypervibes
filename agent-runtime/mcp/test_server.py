"""Lightweight tests for the Vibetrading MCP server helpers.

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
import sys
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
            "vibetrading_mcp_server", MCP_DIR / "server.py"
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


class VibetradingMcpServerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.server = _load_server(
            {
                "VIBETRADING_API_BASE_URL": "http://example.test",
                "VIBETRADING_API_KEY": "vta_test_default",
                "VIBETRADING_AGENT_KEY": "default",
            }
        )

    def test_loads_required_env_from_workspace(self) -> None:
        with mock.patch.dict(
            os.environ,
            {
                "VIBETRADING_API_BASE_URL": "http://example.test/",
                "VIBETRADING_API_KEY": "vta_test_xyz",
                "VIBETRADING_AGENT_KEY": "btc-2",
            },
            clear=True,
        ):
            base_url, api_key, agent_key = self.server._load_config()
        self.assertEqual(base_url, "http://example.test")  # trailing slash stripped
        self.assertEqual(api_key, "vta_test_xyz")
        self.assertEqual(agent_key, "btc-2")

    def test_missing_env_raises_with_clear_message(self) -> None:
        with mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaises(RuntimeError) as ctx:
                self.server._load_config()
        message = str(ctx.exception)
        self.assertIn("VIBETRADING_API_BASE_URL", message)
        self.assertIn("VIBETRADING_API_KEY", message)
        self.assertIn("VIBETRADING_AGENT_KEY", message)

    def test_headers_use_bearer_token(self) -> None:
        with mock.patch.dict(
            os.environ,
            {
                "VIBETRADING_API_BASE_URL": "http://example.test",
                "VIBETRADING_API_KEY": "vta_secret",
                "VIBETRADING_AGENT_KEY": "btc-2",
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
                "VIBETRADING_API_BASE_URL": "http://example.test",
                "VIBETRADING_API_KEY": "k",
                "VIBETRADING_AGENT_KEY": "a",
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

    def test_validation_rejects_invalid_job_kind(self) -> None:
        with self.assertRaises(ValueError):
            self.server.get_job_context("not-a-kind")

    def test_validation_rejects_zero_limit(self) -> None:
        with self.assertRaises(ValueError):
            self.server.get_latest_analysis("BTC", limit=0)

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
                "VIBETRADING_API_BASE_URL": "http://example.test",
                "VIBETRADING_API_KEY": "k",
                "VIBETRADING_AGENT_KEY": "a",
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

    def test_submit_orders_requires_non_empty_list(self) -> None:
        with self.assertRaises(ValueError):
            self.server.submit_orders([])
        with self.assertRaises(ValueError):
            self.server.submit_orders("not-a-list")  # type: ignore[arg-type]

    def test_error_message_redacts_api_key(self) -> None:
        with mock.patch.dict(
            os.environ,
            {
                "VIBETRADING_API_BASE_URL": "http://example.test",
                "VIBETRADING_API_KEY": "vta_super_secret",
                "VIBETRADING_AGENT_KEY": "a",
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

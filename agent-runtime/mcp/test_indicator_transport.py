"""Pinned FastMCP serialization checks, run directly or by test_server.py.

Install requirements.txt first. No HTTP server or OpenCode session is started.
"""

import asyncio
from importlib.metadata import version
import unittest
from unittest import mock

from mcp.server.fastmcp import FastMCP  # Load real MCP before test_server's fake.
from mcp import types

from test_server import _load_server, dense_indicator_run, indicator_definition, indicator_text


class IndicatorTransportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if version("mcp") != "1.28.1":
            raise RuntimeError("Transport contract tests require pinned mcp==1.28.1")
        cls.server = _load_server({
            "HYPERVIBES_API_BASE_URL": "http://example.test",
            "HYPERVIBES_API_KEY": "test-key",
            "HYPERVIBES_AGENT_KEY": "default",
        })
        assert isinstance(cls.server.mcp, FastMCP)
        cls.server.LOGGER.setLevel("WARNING")

    def call(self, tool, args, backend):
        with mock.patch.object(self.server, "_request", return_value=backend):
            projected = getattr(self.server, tool)(**args)
            result = self.wire_call(tool, args)
        self.assertFalse(result.isError)
        text = "\n\n".join(block.text for block in result.content if block.type == "text")
        self.assertEqual(text, indicator_text(projected))
        self.assertEqual(text, self.server._indicator_text(projected))
        self.assertLessEqual(len(text.encode("utf-8")), 24 * 1024)
        self.assertLessEqual(len(text.split("\n")), 1000)
        self.assertEqual(result.structuredContent, {"result": projected} if isinstance(projected, list) else projected)
        return projected

    def wire_call(self, tool, args):
        request = types.CallToolRequest(method="tools/call", params=types.CallToolRequestParams(name=tool, arguments=args))
        handler = self.server.mcp._mcp_server.request_handlers[types.CallToolRequest]
        response = asyncio.run(handler(request))
        # Round-trip the final low-level response, including structured content,
        # through its transport JSON serialization before measuring text blocks.
        return types.CallToolResult.model_validate_json(response.model_dump_json())

    def test_real_tool_content_for_dense_defaults_explicit_and_aggregate_pages(self):
        run = dense_indicator_run(2000, 32, 32)
        for args in ({}, {"bar_limit": 100, "marker_limit": 100}, {"limit": 2}):
            backend = [run, run] if args.get("limit") == 2 else [run]
            result = self.call("get_indicator_results", {"indicator_id": "indicator-id", "timeframe": "15m", **args}, backend)
            for page in result:
                self.assertEqual(page["markers"][0]["bar_index"], 1999)
                self.assertEqual(page["bars"][-1]["bar_index"], 1999)

    def test_real_discovery_content(self):
        run = dense_indicator_run()
        backend = [indicator_definition(run, id=str(index)) for index in range(100)]
        result = self.call("list_indicators", {"limit": 100}, backend)
        self.assertGreater(result["returned_count"], 0)
        self.assertIsNotNone(result["next_offset"])

    def test_real_empty_and_failed_content(self):
        args = {"indicator_id": "indicator-id", "timeframe": "15m"}
        self.call("get_indicator_results", args, [])
        run = dense_indicator_run()
        run.update(status="failed", candle_data=None, plot_data=None, visual_data=None,
                   error_summary="錯" * 10000)
        self.call("get_indicator_results", args, [run])

    def test_real_errors_are_bounded_and_invalid_offsets_do_not_request_data(self):
        args = {"indicator_id": "indicator-id", "timeframe": "15m"}
        for invalid in ({"bar_start": 0}, {"marker_limit": True}, {"limit": 0}):
            with mock.patch.object(self.server, "_request") as request:
                result = self.wire_call("get_indicator_results", {**args, **invalid})
            request.assert_not_called()
            self.assertTrue(result.isError)
            self.assertLess(len("\n\n".join(block.text for block in result.content).encode("utf-8")), 24 * 1024)
        run = dense_indicator_run(1, 1, 1)
        run["visual_data"]["markers"][0]["title"] = "x" * 100000
        with mock.patch.object(self.server, "_request", return_value=[run]):
            result = self.wire_call("get_indicator_results", args)
        self.assertTrue(result.isError)
        text = "\n\n".join(block.text for block in result.content)
        self.assertIn("oversized", text)
        self.assertLess(len(text.encode("utf-8")), 1000)

    def test_memory_transport_leaves_expiry_to_backend(self):
        args = {"scope_kind": "agent", "memory_type": "handoff", "summary": "s", "content": "c"}
        metadata = {
            "handoff_version": 1,
            "stale_after": "2026-10-01T18:05:00Z",
            "valid_for_seconds": 60,
            "expiry_policy": "caller_policy",
        }
        backend = {"id": "memory-id", "expires_at": "2026-10-01T18:30:00Z"}
        with mock.patch.object(self.server, "_request", return_value=backend) as request:
            result = self.wire_call("write_memory", {**args, "metadata": metadata})
        self.assertFalse(result.isError)
        self.assertEqual(request.call_args.kwargs["json_body"]["metadata"], metadata)
        self.assertEqual(result.structuredContent["expires_at"], backend["expires_at"])


if __name__ == "__main__":
    unittest.main()

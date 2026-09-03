import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "coding_validate", ROOT / "agent-runtime" / "coding_validate.py"
)
assert SPEC is not None
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(MODULE)


ANALYZER = '''
import argparse, json, os

parser = argparse.ArgumentParser()
parser.add_argument("--symbol")
parser.add_argument("--timeframe")
parser.add_argument("--boundary-ms", type=int)
parser.add_argument("--input")
parser.add_argument("--output")
args = parser.parse_args()
data = json.loads(open(args.input).read())
if data["symbol"] != args.symbol or data["timeframe"] != args.timeframe:
    raise ValueError("CLI/input context mismatch")
interval = data["interval_ms"]
closed = [c for c in data["candles"] if c["timestamp_ms"] + interval <= args.boundary_ms]
closed.sort(key=lambda candle: candle["timestamp_ms"])
result = {
    "symbol": args.symbol,
    "timeframe": args.timeframe,
    "boundary_ms": args.boundary_ms,
    "source_range": {"count": len(closed)},
    "code_version": "test",
    "measurements": {
        "closed_count": len(closed),
        "last_open": closed[-1]["open"] if closed else None,
        "last_close": closed[-1]["close"] if closed else None,
    },
    "warnings": [],
}
os.makedirs(os.path.dirname(args.output), exist_ok=True)
open(args.output, "w").write(json.dumps(result, sort_keys=True))
'''

SLOW_ANALYZER = ANALYZER.replace(
    'open(args.output, "w").write(json.dumps(result, sort_keys=True))',
    'import time\nopen(args.output, "w").write(json.dumps(result, sort_keys=True))\ntime.sleep(2)',
)


def manifest_for(entrypoint: str, target_id: str = "trend") -> str:
    return json.dumps(
        {
            "schema_version": 1,
            "package_version": "test",
            "tools": [
                {
                    "id": target_id,
                    "description": "Test analyzer",
                    "entrypoint": entrypoint,
                    "input_kind": "ohlcv",
                    "supported_timeframes": list(MODULE.INTERVAL_MS),
                    "minimum_candles": 1,
                    "required_arguments": [
                        "symbol", "timeframe", "boundary_ms", "input", "output"
                    ],
                    "output_schema": "hypervibes.quantitative.v1",
                    "version": "1",
                }
            ],
        }
    )


MANIFEST = manifest_for("strategies/trend.py")


def write_candidate(user: Path, analyzer: str) -> None:
    (user / "strategies").mkdir(parents=True, exist_ok=True)
    (user / "strategies/trend.py").write_text(analyzer)
    (user / "manifest.json").write_text(MANIFEST)


class CodingValidatorTests(unittest.TestCase):
    def test_fixture_suite_passes_for_arbitrary_target_names(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            user = workspace / "scripts/user"
            user.mkdir(parents=True)
            write_candidate(user, ANALYZER)
            result = MODULE.validate(workspace)
            self.assertTrue(result["ok"], result)
            self.assertIn("unit tests not provided", result["checks"])
            self.assertIn("trend: open-candle rejection", result["checks"])
            self.assertIn("trend: future-candle causality", result["checks"])
            self.assertIn("trend: candle-order invariance", result["checks"])
            self.assertIn("trend: known signal semantics", result["checks"])
            self.assertIn("trend: eligible-candle sensitivity", result["checks"])
            self.assertIn("trend: declared timeframes", result["checks"])
            self.assertFalse(list(user.rglob("*.pyc")))

    def test_every_declared_target_receives_the_complete_suite(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            user = workspace / "scripts/user"
            (user / "strategies").mkdir(parents=True)
            (user / "strategies/trend.py").write_text(ANALYZER)
            (user / "strategies/mean_reversion.py").write_text(ANALYZER)
            (user / "manifest.json").write_text(json.dumps({
                "schema_version": 1,
                "package_version": "test",
                "tools": [
                    {
                        "id": "trend",
                        "description": "Trend target",
                        "entrypoint": "strategies/trend.py",
                        "input_kind": "ohlcv",
                        "supported_timeframes": ["15m", "1h"],
                        "minimum_candles": 1,
                        "required_arguments": [
                            "symbol", "timeframe", "boundary_ms", "input", "output"
                        ],
                        "output_schema": "hypervibes.quantitative.v1",
                        "version": "1",
                    },
                    {
                        "id": "mean_reversion",
                        "description": "Mean reversion target",
                        "entrypoint": "strategies/mean_reversion.py",
                        "input_kind": "ohlcv",
                        "supported_timeframes": ["1d"],
                        "minimum_candles": 1,
                        "required_arguments": [
                            "symbol", "timeframe", "boundary_ms", "input", "output"
                        ],
                        "output_schema": "hypervibes.quantitative.v1",
                        "version": "1",
                    },
                ],
            }))
            result = MODULE.validate(workspace)
            self.assertTrue(result["ok"], result)
            self.assertIn("trend: eligible-candle sensitivity", result["checks"])
            self.assertIn("mean_reversion: eligible-candle sensitivity", result["checks"])
            self.assertIn("trend: declared timeframes", result["checks"])
            self.assertIn("mean_reversion: declared timeframes", result["checks"])

    def test_failure_identifies_its_target_id(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            user = workspace / "scripts/user"
            (user / "strategies").mkdir(parents=True)
            (user / "strategies/trend.py").write_text(ANALYZER)
            (user / "strategies/broken.py").write_text(
                ANALYZER.replace(
                    '"last_close": closed[-1]["close"] if closed else None,',
                    '"last_close": 1,',
                )
            )
            (user / "manifest.json").write_text(json.dumps({
                "schema_version": 1,
                "package_version": "test",
                "tools": [
                    {
                        "id": "trend",
                        "description": "Trend target",
                        "entrypoint": "strategies/trend.py",
                        "input_kind": "ohlcv",
                        "supported_timeframes": ["15m"],
                        "minimum_candles": 1,
                        "required_arguments": [
                            "symbol", "timeframe", "boundary_ms", "input", "output"
                        ],
                        "output_schema": "hypervibes.quantitative.v1",
                        "version": "1",
                    },
                    {
                        "id": "broken",
                        "description": "Broken target",
                        "entrypoint": "strategies/broken.py",
                        "input_kind": "ohlcv",
                        "supported_timeframes": ["15m"],
                        "minimum_candles": 1,
                        "required_arguments": [
                            "symbol", "timeframe", "boundary_ms", "input", "output"
                        ],
                        "output_schema": "hypervibes.quantitative.v1",
                        "version": "1",
                    },
                ],
            }))
            result = MODULE.validate(workspace)
            self.assertFalse(result["ok"])
            self.assertTrue(
                any("target 'broken'" in check for check in result["checks"]),
                result["checks"],
            )
            self.assertTrue(
                any("eligible-candle sensitivity" in check for check in result["checks"]),
                result["checks"],
            )

    def test_manifest_requires_at_least_one_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            user = workspace / "scripts/user"
            (user / "strategies").mkdir(parents=True)
            (user / "strategies/trend.py").write_text(ANALYZER)
            (user / "manifest.json").write_text(json.dumps({
                "schema_version": 1,
                "package_version": "test",
                "tools": [],
            }))
            result = MODULE.validate(workspace)
            self.assertFalse(result["ok"])
            self.assertIn("package manifest schema failed", result["checks"])

    def test_manifest_rejects_duplicate_target_ids(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            user = workspace / "scripts/user"
            (user / "strategies").mkdir(parents=True)
            (user / "strategies/trend.py").write_text(ANALYZER)
            (user / "strategies/other.py").write_text(ANALYZER)
            (user / "manifest.json").write_text(json.dumps({
                "schema_version": 1,
                "package_version": "test",
                "tools": [
                    {
                        "id": "trend",
                        "description": "Trend target",
                        "entrypoint": "strategies/trend.py",
                        "input_kind": "ohlcv",
                        "supported_timeframes": ["15m"],
                        "minimum_candles": 1,
                        "required_arguments": [
                            "symbol", "timeframe", "boundary_ms", "input", "output"
                        ],
                        "output_schema": "hypervibes.quantitative.v1",
                        "version": "1",
                    },
                    {
                        "id": "trend",
                        "description": "Duplicate target",
                        "entrypoint": "strategies/other.py",
                        "input_kind": "ohlcv",
                        "supported_timeframes": ["15m"],
                        "minimum_candles": 1,
                        "required_arguments": [
                            "symbol", "timeframe", "boundary_ms", "input", "output"
                        ],
                        "output_schema": "hypervibes.quantitative.v1",
                        "version": "1",
                    },
                ],
            }))
            result = MODULE.validate(workspace)
            self.assertFalse(result["ok"])
            self.assertIn("declaration is invalid", result["checks"][0])

    def test_manifest_rejects_unknown_declared_timeframes(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            user = workspace / "scripts/user"
            (user / "strategies").mkdir(parents=True)
            (user / "strategies/trend.py").write_text(ANALYZER)
            (user / "manifest.json").write_text(
                manifest_for("strategies/trend.py").replace('"15m"', '"45m"')
            )
            result = MODULE.validate(workspace)
            self.assertFalse(result["ok"])
            self.assertTrue(
                any("declaration is invalid" in check for check in result["checks"]),
                result["checks"],
            )

    def test_forbidden_source_fails_validation(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            user = workspace / "scripts/user"
            user.mkdir(parents=True)
            write_candidate(user, "import subprocess\n")
            self.assertFalse(MODULE.validate(workspace)["ok"])

    def test_optional_candidate_tests_run_when_present(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            user = workspace / "scripts/user"
            tests = user / "tests"
            tests.mkdir(parents=True)
            write_candidate(user, ANALYZER)
            (tests / "test_failure.py").write_text(
                "import unittest\n"
                "class Failure(unittest.TestCase):\n"
                "    def test_failure(self):\n"
                "        self.fail('expected')\n"
            )
            result = MODULE.validate(workspace)
            self.assertFalse(result["ok"])
            self.assertIn("unit tests failed", result["checks"])
            self.assertIn("expected", result["diagnostics"][0])

    def test_analyzer_that_ignores_eligible_values_fails_sensitivity(self):
        inert_analyzer = ANALYZER.replace(
            '"last_close": closed[-1]["close"] if closed else None,',
            '"last_close": 1,',
        )
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            user = workspace / "scripts/user"
            user.mkdir(parents=True)
            write_candidate(user, inert_analyzer)
            result = MODULE.validate(workspace)
            self.assertFalse(result["ok"])
            self.assertTrue(
                any("eligible-candle sensitivity" in check for check in result["checks"]),
                result["checks"],
            )

    def test_analyzer_must_create_output_parent(self):
        analyzer = ANALYZER.replace(
            'os.makedirs(os.path.dirname(args.output), exist_ok=True)\n', ""
        )
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            user = workspace / "scripts/user"
            user.mkdir(parents=True)
            write_candidate(user, analyzer)
            result = MODULE.validate(workspace)
            self.assertFalse(result["ok"])
            self.assertTrue(
                any("output schema failed" in check for check in result["checks"]),
                result["checks"],
            )

    def test_analyzer_must_be_order_invariant(self):
        analyzer = ANALYZER.replace(
            'closed.sort(key=lambda candle: candle["timestamp_ms"])\n', ""
        )
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            user = workspace / "scripts/user"
            user.mkdir(parents=True)
            write_candidate(user, analyzer)
            result = MODULE.validate(workspace)
            self.assertFalse(result["ok"])
            self.assertTrue(
                any("candle-order invariance failed" in check for check in result["checks"])
            )

    def test_known_candle_body_signal_uses_open_and_close(self):
        analyzer = ANALYZER.replace(
            '"warnings": [],',
            '"warnings": [],\n'
            '    "signals": [{"name": "last_candle_body", "state": "up"}],',
        )
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            user = workspace / "scripts/user"
            user.mkdir(parents=True)
            write_candidate(user, analyzer)
            result = MODULE.validate(workspace)
            self.assertFalse(result["ok"])
            self.assertTrue(
                any(
                    "last_candle_body state must compare last_close with last_open" in check
                    for check in result["checks"]
                ),
                result["checks"],
            )

    def test_undeclared_helper_files_participate_in_compilation_and_scan(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            user = workspace / "scripts/user"
            user.mkdir(parents=True)
            write_candidate(user, ANALYZER)
            (user / "shared").mkdir()
            (user / "shared/indicators.py").write_text("import subprocess\n")
            result = MODULE.validate(workspace)
            self.assertFalse(result["ok"])
            self.assertIn("forbidden source in indicators.py", result["checks"])


if __name__ == "__main__":
    unittest.main()
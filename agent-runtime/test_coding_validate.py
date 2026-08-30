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
closed = [c for c in data["candles"] if c["timestamp_ms"] + interval < args.boundary_ms]
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

MANIFEST = json.dumps(
    {
        "schema_version": 1,
        "package_version": "test",
        "tools": [
            {
                "id": "analyze",
                "description": "Test analyzer",
                "entrypoint": "analyze.py",
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


def write_candidate(user: Path, analyzer: str) -> None:
    (user / "analyze.py").write_text(analyzer)
    (user / "manifest.json").write_text(MANIFEST)


class CodingValidatorTests(unittest.TestCase):
    def test_canonical_fixture_passes_open_candle_and_causality(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            user = workspace / "scripts/user"
            user.mkdir(parents=True)
            write_candidate(user, ANALYZER)
            result = MODULE.validate(workspace)
            self.assertTrue(result["ok"], result)
            self.assertIn("unit tests not provided", result["checks"])
            self.assertIn("open-candle rejection", result["checks"])
            self.assertIn("future-candle causality", result["checks"])
            self.assertIn("candle-order invariance", result["checks"])
            self.assertIn("known signal semantics", result["checks"])
            self.assertIn("eligible-candle sensitivity", result["checks"])
            self.assertIn("supported intervals", result["checks"])
            self.assertFalse(list(user.rglob("*.pyc")))

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
            self.assertIn("eligible-candle sensitivity failed", result["checks"])

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
            self.assertIn("canonical output schema failed", result["checks"])

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
            self.assertIn(
                "last_candle_body state must compare last_close with last_open",
                result["checks"],
            )


if __name__ == "__main__":
    unittest.main()

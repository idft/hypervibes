#!/usr/bin/env python3
"""Fixed validation for an analysis-coding candidate."""

from __future__ import annotations

import argparse
import compileall
import json
import math
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


FORBIDDEN_SOURCE = re.compile(
    r"https?://|/workspaces/agents/|subprocess|os\.system|pip\s+install|package\.json"
)
INTERVAL_MS = {
    "1m": 60_000,
    "3m": 180_000,
    "5m": 300_000,
    "15m": 900_000,
    "30m": 1_800_000,
    "1h": 3_600_000,
    "2h": 7_200_000,
    "4h": 14_400_000,
    "8h": 28_800_000,
    "12h": 43_200_000,
    "1d": 86_400_000,
    "3d": 259_200_000,
    "1w": 604_800_000,
    "1M": 2_592_000_000,
}
BOUNDARY_MS = 1_700_000_900_000
MAX_DIAGNOSTIC_CHARS = 4_000


def _diagnostic(completed: subprocess.CompletedProcess[Any]) -> str:
    text = "\n".join(
        part.strip()
        for part in (completed.stdout or "", completed.stderr or "")
        if part.strip()
    )
    return text[-MAX_DIAGNOSTIC_CHARS:]


def _failure(
    checks: list[str], message: str, completed: subprocess.CompletedProcess[Any] | None = None
) -> dict[str, object]:
    result: dict[str, object] = {"ok": False, "checks": checks + [message]}
    if completed is not None:
        diagnostic = _diagnostic(completed)
        if diagnostic:
            result["diagnostics"] = [diagnostic]
    return result


def _first_difference(expected: Any, actual: Any, path: str = "$") -> str:
    if type(expected) is not type(actual):
        return path
    if isinstance(expected, dict):
        for key in sorted(set(expected) | set(actual)):
            child = f"{path}.{key}"
            if key not in expected or key not in actual:
                return child
            difference = _first_difference(expected[key], actual[key], child)
            if difference:
                return difference
        return ""
    if isinstance(expected, list):
        if len(expected) != len(actual):
            return f"{path}.length"
        for index, (expected_item, actual_item) in enumerate(zip(expected, actual)):
            difference = _first_difference(
                expected_item, actual_item, f"{path}[{index}]"
            )
            if difference:
                return difference
        return ""
    return "" if expected == actual else path


def _invariance_failure(
    checks: list[str], message: str, expected: Any, actual: Any
) -> dict[str, object]:
    difference = _first_difference(expected, actual) or "$"
    return _failure(checks, f"{message}; output differs at {difference}")


def _all_finite(value: Any) -> bool:
    if isinstance(value, float):
        return math.isfinite(value)
    if isinstance(value, dict):
        return all(_all_finite(item) for item in value.values())
    if isinstance(value, list):
        return all(_all_finite(item) for item in value)
    return True


def _fixture_for_interval(timeframe: str, interval_ms: int) -> dict[str, Any]:
    def candle(start_ms: int, close: float) -> dict[str, int | float]:
        return {
            "timestamp_ms": start_ms,
            "open": close - 1,
            "high": close + 1,
            "low": close - 2,
            "close": close,
            "volume": 10,
        }

    return {
        "symbol": "BTC",
        "timeframe": timeframe,
        "interval_ms": interval_ms,
        "candles": [
            candle(BOUNDARY_MS - 3 * interval_ms, 101),
            candle(BOUNDARY_MS - interval_ms, 102),
            candle(BOUNDARY_MS, 103),
        ],
    }


def _run_analyzer(
    implementation: Path,
    workspace: Path,
    run_root: Path,
    fixture_data: dict[str, Any],
    *,
    symbol: str = "BTC",
    timeframe: str | None = None,
    suffix: str,
    environment: dict[str, str],
) -> tuple[subprocess.CompletedProcess[Any], dict[str, Any] | None]:
    timeframe = timeframe or str(fixture_data["timeframe"])
    input_path = run_root / f"{suffix}-input.json"
    output_path = run_root / "outputs" / f"{suffix}-output.json"
    input_path.write_text(json.dumps(fixture_data, sort_keys=True))
    command = [
        sys.executable,
        str(implementation),
        "--symbol",
        symbol,
        "--timeframe",
        timeframe,
        "--boundary-ms",
        str(BOUNDARY_MS),
        "--input",
        str(input_path),
        "--output",
        str(output_path),
    ]
    completed = subprocess.run(
        command,
        cwd=workspace,
        capture_output=True,
        text=True,
        timeout=30,
        env=environment,
    )
    if completed.returncode != 0 or not output_path.is_file():
        return completed, None
    try:
        output = json.loads(output_path.read_text(), parse_constant=lambda value: (_ for _ in ()).throw(ValueError(value)))
    except (ValueError, json.JSONDecodeError):
        return completed, None
    return completed, output


def _validate_output(
    output: dict[str, Any] | None,
    *,
    timeframe: str,
    expected_count: int,
) -> str | None:
    required = {
        "symbol",
        "timeframe",
        "boundary_ms",
        "source_range",
        "code_version",
        "measurements",
        "warnings",
    }
    if not isinstance(output, dict) or not required.issubset(output):
        return "canonical output schema failed"
    if output["symbol"] != "BTC" or output["timeframe"] != timeframe:
        return "canonical output context failed"
    if output["boundary_ms"] != BOUNDARY_MS:
        return "canonical output boundary failed"
    if not isinstance(output["code_version"], str) or not output["code_version"].strip():
        return "canonical code_version failed"
    if not isinstance(output["source_range"], dict):
        return "canonical source_range type failed"
    if output["source_range"].get("count") != expected_count:
        return "canonical closed-candle count failed"
    if not isinstance(output["measurements"], dict) or not output["measurements"]:
        return "canonical measurements must be non-empty"
    if not isinstance(output["warnings"], list):
        return "canonical warnings type failed"
    if "signals" in output and not isinstance(output["signals"], (dict, list)):
        return "canonical signals type failed"
    if not _all_finite(output):
        return "canonical output contains non-finite numbers"
    return None


def _validate_known_signals(output: dict[str, Any] | None) -> str | None:
    if not isinstance(output, dict) or not isinstance(output.get("signals"), list):
        return None
    measurements = output.get("measurements")
    if not isinstance(measurements, dict):
        return None
    last_open = measurements.get("last_open")
    last_close = measurements.get("last_close")
    if not isinstance(last_open, (int, float)) or not isinstance(
        last_close, (int, float)
    ):
        return None
    expected_state = (
        "up" if last_close > last_open else "down" if last_close < last_open else "flat"
    )
    for signal in output["signals"]:
        if not isinstance(signal, dict):
            continue
        if signal.get("name") != "last_candle_body" and signal.get("type") != "last_candle_body":
            continue
        if signal.get("state") != expected_state:
            return "last_candle_body state must compare last_close with last_open"
    return None


def validate(workspace: Path) -> dict[str, object]:
    user = (workspace / "scripts" / "user").resolve()
    if not user.is_dir():
        return {"ok": False, "checks": ["scripts/user is missing"]}

    checks: list[str] = []
    implementation = user / "analyze.py"
    fixture = Path(__file__).with_name("coding_fixture.json")
    if not implementation.is_file() or not fixture.is_file():
        return _failure(checks, "canonical implementation or fixture is missing")

    try:
        fixture_data = json.loads(fixture.read_text())
        with tempfile.TemporaryDirectory(prefix="coding-validate-") as temporary:
            run_root = Path(temporary)
            pycache_root = run_root / "pycache"
            previous_pycache_prefix = sys.pycache_prefix
            sys.pycache_prefix = str(pycache_root)
            try:
                if not compileall.compile_dir(str(user), quiet=1):
                    return _failure(checks, "python compilation failed")
            finally:
                sys.pycache_prefix = previous_pycache_prefix
            checks.append("compile")

            environment = os.environ.copy()
            environment["PYTHONHASHSEED"] = "0"
            environment["PYTHONPYCACHEPREFIX"] = str(pycache_root)

            tests_root = user / "tests"
            test_files = list(tests_root.rglob("test*.py")) if tests_root.is_dir() else []
            if test_files:
                completed = subprocess.run(
                    [sys.executable, "-m", "unittest", "discover", "-s", str(tests_root)],
                    cwd=user,
                    capture_output=True,
                    text=True,
                    timeout=30,
                    env=environment,
                )
                if completed.returncode != 0:
                    return _failure(checks, "unit tests failed", completed)
                checks.append("unit")
            else:
                checks.append("unit tests not provided")

            for path in sorted(user.rglob("*")):
                if path.is_file() and path.suffix in {".py", ".json", ".md"}:
                    try:
                        text = path.read_text()
                    except UnicodeDecodeError:
                        return _failure(checks, f"non-text file: {path.name}")
                    if FORBIDDEN_SOURCE.search(text):
                        return _failure(checks, f"forbidden source in {path.name}")
            checks.append("source scan")

            completed, baseline = _run_analyzer(
                implementation,
                workspace,
                run_root,
                fixture_data,
                suffix="baseline",
                environment=environment,
            )
            error = _validate_output(baseline, timeframe="15m", expected_count=1)
            if completed.returncode != 0 or error is not None:
                return _failure(checks, error or "canonical CLI failed", completed)
            checks.append("CLI, context, and output schema")

            _, repeated = _run_analyzer(
                implementation,
                workspace,
                run_root,
                fixture_data,
                suffix="repeat",
                environment=environment,
            )
            if repeated != baseline:
                return _failure(checks, "deterministic output failed")
            checks.append("deterministic output")

            open_data = dict(fixture_data)
            open_data["candles"] = list(fixture_data["candles"]) + [
                {
                    "timestamp_ms": BOUNDARY_MS - 450_000,
                    "open": 1000,
                    "high": 1001,
                    "low": 999,
                    "close": 1000,
                    "volume": 999,
                }
            ]
            _, open_output = _run_analyzer(
                implementation,
                workspace,
                run_root,
                open_data,
                suffix="open",
                environment=environment,
            )
            if open_output != baseline:
                return _invariance_failure(
                    checks, "open-candle rejection failed", baseline, open_output
                )
            checks.append("open-candle rejection")

            future_data = dict(fixture_data)
            future_data["candles"] = list(fixture_data["candles"]) + [
                {
                    "timestamp_ms": BOUNDARY_MS + 1_800_000,
                    "open": 1000,
                    "high": 1001,
                    "low": 999,
                    "close": 1000,
                    "volume": 999,
                }
            ]
            _, future_output = _run_analyzer(
                implementation,
                workspace,
                run_root,
                future_data,
                suffix="future",
                environment=environment,
            )
            if future_output != baseline:
                return _invariance_failure(
                    checks, "future-candle causality failed", baseline, future_output
                )
            checks.append("future-candle causality")

            ordering_data = _fixture_for_interval("15m", INTERVAL_MS["15m"])
            ordering_data["candles"] = [
                {
                    "timestamp_ms": BOUNDARY_MS - 4 * INTERVAL_MS["15m"],
                    "open": 99,
                    "high": 101,
                    "low": 98,
                    "close": 100,
                    "volume": 9,
                },
                {
                    "timestamp_ms": BOUNDARY_MS - 3 * INTERVAL_MS["15m"],
                    "open": 100,
                    "high": 102,
                    "low": 99,
                    "close": 101,
                    "volume": 10,
                },
                ordering_data["candles"][1],
            ]
            _, ordered_output = _run_analyzer(
                implementation,
                workspace,
                run_root,
                ordering_data,
                suffix="ordered",
                environment=environment,
            )
            shuffled_data = json.loads(json.dumps(ordering_data))
            shuffled_data["candles"] = list(reversed(shuffled_data["candles"]))
            _, shuffled_output = _run_analyzer(
                implementation,
                workspace,
                run_root,
                shuffled_data,
                suffix="shuffled",
                environment=environment,
            )
            if ordered_output is None or shuffled_output != ordered_output:
                return _invariance_failure(
                    checks,
                    "candle-order invariance failed",
                    ordered_output,
                    shuffled_output,
                )
            checks.append("candle-order invariance")

            body_data = _fixture_for_interval("15m", INTERVAL_MS["15m"])
            body_data["candles"] = [
                {
                    "timestamp_ms": BOUNDARY_MS - 4 * INTERVAL_MS["15m"],
                    "open": 99,
                    "high": 101,
                    "low": 98,
                    "close": 100,
                    "volume": 9,
                },
                {
                    "timestamp_ms": BOUNDARY_MS - 3 * INTERVAL_MS["15m"],
                    "open": 110,
                    "high": 111,
                    "low": 104,
                    "close": 105,
                    "volume": 10,
                },
            ]
            body_run, body_output = _run_analyzer(
                implementation,
                workspace,
                run_root,
                body_data,
                suffix="body-signal",
                environment=environment,
            )
            error = _validate_output(body_output, timeframe="15m", expected_count=2)
            if body_run.returncode != 0 or error is not None:
                return _failure(checks, error or "body-signal fixture failed", body_run)
            error = _validate_known_signals(body_output)
            if error is not None:
                return _failure(checks, error)
            checks.append("known signal semantics")

            changed_data = json.loads(json.dumps(fixture_data))
            changed_data["candles"][0]["close"] = 150
            changed_data["candles"][0]["high"] = 151
            _, changed_output = _run_analyzer(
                implementation,
                workspace,
                run_root,
                changed_data,
                suffix="sensitivity",
                environment=environment,
            )
            if changed_output is None or changed_output == baseline:
                return _failure(checks, "eligible-candle sensitivity failed")
            checks.append("eligible-candle sensitivity")

            mismatch, _ = _run_analyzer(
                implementation,
                workspace,
                run_root,
                fixture_data,
                symbol="ETH",
                suffix="context-mismatch",
                environment=environment,
            )
            if mismatch.returncode == 0:
                return _failure(checks, "CLI/input context mismatch was accepted")
            checks.append("CLI/input context mismatch rejection")

            for timeframe, interval_ms in INTERVAL_MS.items():
                interval_fixture = _fixture_for_interval(timeframe, interval_ms)
                interval_run, interval_output = _run_analyzer(
                    implementation,
                    workspace,
                    run_root,
                    interval_fixture,
                    suffix=f"interval-{timeframe}",
                    environment=environment,
                )
                error = _validate_output(
                    interval_output,
                    timeframe=timeframe,
                    expected_count=1,
                )
                if interval_run.returncode != 0 or error is not None:
                    return _failure(
                        checks,
                        f"supported interval {timeframe} failed: {error or 'execution failed'}",
                        interval_run,
                    )
            checks.append("supported intervals")
    except (OSError, ValueError, json.JSONDecodeError, subprocess.SubprocessError) as error:
        return _failure(checks, f"canonical validation error: {error}")
    return {"ok": True, "checks": checks}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--workspace", required=True)
    args = parser.parse_args()
    result = validate(Path(args.workspace))
    print(json.dumps(result, sort_keys=True))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

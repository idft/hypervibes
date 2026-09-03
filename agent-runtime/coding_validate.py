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
    r"https?://|/workspaces/|subprocess|os\.system|pip\s+install|package\.json"
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
MANIFEST_FILENAME = "manifest.json"
REQUIRED_TOOL_ARGUMENTS = {
    "symbol",
    "timeframe",
    "boundary_ms",
    "input",
    "output",
}


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


def _run_target(
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
    try:
        completed = subprocess.run(
            command,
            cwd=workspace,
            capture_output=True,
            text=True,
            timeout=30,
            env=environment,
        )
    except subprocess.TimeoutExpired as error:
        stdout = error.stdout if isinstance(error.stdout, str) else ""
        stderr = error.stderr if isinstance(error.stderr, str) else ""
        return subprocess.CompletedProcess(command, 124, stdout, stderr), None
    except OSError as error:
        return subprocess.CompletedProcess(command, 127, "", str(error)), None
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
        return "output schema failed"
    if output["symbol"] != "BTC" or output["timeframe"] != timeframe:
        return "output context failed"
    if output["boundary_ms"] != BOUNDARY_MS:
        return "output boundary failed"
    if not isinstance(output["code_version"], str) or not output["code_version"].strip():
        return "code_version failed"
    if not isinstance(output["source_range"], dict):
        return "source_range type failed"
    if output["source_range"].get("count") != expected_count:
        return "closed-candle count failed"
    if not isinstance(output["measurements"], dict) or not output["measurements"]:
        return "measurements must be non-empty"
    if not isinstance(output["warnings"], list):
        return "warnings type failed"
    if "signals" in output and not isinstance(output["signals"], (dict, list)):
        return "signals type failed"
    if not _all_finite(output):
        return "output contains non-finite numbers"
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


def _validate_manifest(
    user: Path,
) -> tuple[list[tuple[str, Path, list[str]]], str | None]:
    try:
        manifest = json.loads((user / MANIFEST_FILENAME).read_text())
    except (OSError, json.JSONDecodeError):
        return [], "package manifest is missing or invalid"
    if (
        not isinstance(manifest, dict)
        or manifest.get("schema_version") != 1
        or not isinstance(manifest.get("package_version"), str)
        or not manifest["package_version"].strip()
        or not isinstance(manifest.get("tools"), list)
        or not manifest["tools"]
    ):
        return [], "package manifest schema failed"
    targets: list[tuple[str, Path, list[str]]] = []
    seen_ids: set[str] = set()
    for tool in manifest["tools"]:
        if not isinstance(tool, dict):
            return [], "package manifest contains an invalid tool"
        target_id = tool.get("id")
        entrypoint = tool.get("entrypoint")
        entrypoint_path = Path(entrypoint) if isinstance(entrypoint, str) else None
        required_arguments = tool.get("required_arguments")
        supported_timeframes = tool.get("supported_timeframes")
        if (
            not isinstance(target_id, str)
            or not target_id.strip()
            or any(character.isspace() for character in target_id)
            or target_id in seen_ids
            or not isinstance(entrypoint, str)
            or entrypoint_path is None
            or entrypoint_path.is_absolute()
            or ".." in entrypoint_path.parts
            or "\\" in entrypoint
            or entrypoint_path.suffix.lower() != ".py"
            or not isinstance(tool.get("description"), str)
            or not tool["description"].strip()
            or tool.get("input_kind") != "ohlcv"
            or tool.get("output_schema") != "hypervibes.quantitative.v1"
            or not isinstance(required_arguments, list)
            or any(not isinstance(argument, str) for argument in required_arguments)
            or set(required_arguments) != REQUIRED_TOOL_ARGUMENTS
            or not isinstance(tool.get("version"), str)
            or not tool["version"].strip()
            or isinstance(tool.get("minimum_candles"), bool)
            or not isinstance(tool.get("minimum_candles"), int)
            or tool["minimum_candles"] < 1
            or not isinstance(supported_timeframes, list)
            or not supported_timeframes
            or any(not isinstance(timeframe, str) for timeframe in supported_timeframes)
            or any(
                timeframe not in INTERVAL_MS for timeframe in supported_timeframes
            )
        ):
            return [], f"package manifest target {target_id!r} declaration is invalid"
        implementation = (user / entrypoint_path).resolve()
        if not implementation.is_relative_to(user.resolve()) or not implementation.is_file():
            return [], f"package manifest target {target_id!r} entrypoint is missing"
        seen_ids.add(target_id)
        targets.append((target_id, implementation, list(supported_timeframes)))
    return targets, None


def _validate_target_suite(
    checks: list[str],
    target_id: str,
    implementation: Path,
    workspace: Path,
    run_root: Path,
    fixture_data: dict[str, Any],
    declared_timeframes: list[str],
    environment: dict[str, str],
) -> dict[str, object] | None:
    def failure(message: str, completed: subprocess.CompletedProcess[Any] | None = None) -> dict[str, object]:
        return _failure(checks, f"target {target_id!r}: {message}", completed)

    def invariance_failure(message: str, expected: Any, actual: Any) -> dict[str, object]:
        difference = _first_difference(expected, actual) or "$"
        return _failure(
            checks, f"target {target_id!r}: {message}; output differs at {difference}"
        )

    baseline_timeframe = declared_timeframes[0]
    baseline_data = (
        fixture_data
        if baseline_timeframe == fixture_data.get("timeframe")
        else _fixture_for_interval(baseline_timeframe, INTERVAL_MS[baseline_timeframe])
    )
    completed, baseline = _run_target(
        implementation,
        workspace,
        run_root,
        baseline_data,
        timeframe=baseline_timeframe,
        suffix=f"{target_id}-baseline",
        environment=environment,
    )
    error = _validate_output(baseline, timeframe=baseline_timeframe, expected_count=2)
    if completed.returncode != 0 or error is not None:
        return failure(error or "CLI failed", completed)
    checks.append(f"{target_id}: CLI, context, and output schema")

    _, repeated = _run_target(
        implementation,
        workspace,
        run_root,
        baseline_data,
        timeframe=baseline_timeframe,
        suffix=f"{target_id}-repeat",
        environment=environment,
    )
    if repeated != baseline:
        return failure("deterministic output failed")
    checks.append(f"{target_id}: deterministic output")

    interval_ms = INTERVAL_MS[baseline_timeframe]
    open_data = dict(baseline_data)
    open_data["candles"] = list(baseline_data["candles"]) + [
        {
            "timestamp_ms": BOUNDARY_MS - interval_ms // 2,
            "open": 1000,
            "high": 1001,
            "low": 999,
            "close": 1000,
            "volume": 999,
        }
    ]
    _, open_output = _run_target(
        implementation,
        workspace,
        run_root,
        open_data,
        timeframe=baseline_timeframe,
        suffix=f"{target_id}-open",
        environment=environment,
    )
    if open_output != baseline:
        return invariance_failure("open-candle rejection failed", baseline, open_output)
    checks.append(f"{target_id}: open-candle rejection")

    future_data = dict(baseline_data)
    future_data["candles"] = list(baseline_data["candles"]) + [
        {
            "timestamp_ms": BOUNDARY_MS + interval_ms,
            "open": 1000,
            "high": 1001,
            "low": 999,
            "close": 1000,
            "volume": 999,
        }
    ]
    _, future_output = _run_target(
        implementation,
        workspace,
        run_root,
        future_data,
        timeframe=baseline_timeframe,
        suffix=f"{target_id}-future",
        environment=environment,
    )
    if future_output != baseline:
        return invariance_failure("future-candle causality failed", baseline, future_output)
    checks.append(f"{target_id}: future-candle causality")

    ordering_data = _fixture_for_interval(baseline_timeframe, interval_ms)
    ordering_data["candles"] = [
        dict(ordering_data["candles"][0], timestamp_ms=BOUNDARY_MS - 4 * interval_ms, close=100),
        dict(ordering_data["candles"][0], timestamp_ms=BOUNDARY_MS - 3 * interval_ms, close=101),
        ordering_data["candles"][1],
    ]
    _, ordered_output = _run_target(
        implementation,
        workspace,
        run_root,
        ordering_data,
        timeframe=baseline_timeframe,
        suffix=f"{target_id}-ordered",
        environment=environment,
    )
    shuffled_data = json.loads(json.dumps(ordering_data))
    shuffled_data["candles"] = list(reversed(shuffled_data["candles"]))
    _, shuffled_output = _run_target(
        implementation,
        workspace,
        run_root,
        shuffled_data,
        timeframe=baseline_timeframe,
        suffix=f"{target_id}-shuffled",
        environment=environment,
    )
    if ordered_output is None or shuffled_output != ordered_output:
        return invariance_failure(
            "candle-order invariance failed",
            ordered_output,
            shuffled_output,
        )
    checks.append(f"{target_id}: candle-order invariance")

    body_data = _fixture_for_interval(baseline_timeframe, interval_ms)
    body_data["candles"] = [
        dict(body_data["candles"][0], timestamp_ms=BOUNDARY_MS - 4 * interval_ms, close=100),
        dict(body_data["candles"][0], timestamp_ms=BOUNDARY_MS - 3 * interval_ms, open=110, high=111, low=104, close=105),
    ]
    body_run, body_output = _run_target(
        implementation,
        workspace,
        run_root,
        body_data,
        timeframe=baseline_timeframe,
        suffix=f"{target_id}-body-signal",
        environment=environment,
    )
    error = _validate_output(body_output, timeframe=baseline_timeframe, expected_count=2)
    if body_run.returncode != 0 or error is not None:
        return failure(error or "body-signal fixture failed", body_run)
    error = _validate_known_signals(body_output)
    if error is not None:
        return failure(error)
    checks.append(f"{target_id}: known signal semantics")

    changed_data = json.loads(json.dumps(baseline_data))
    changed_data["candles"][1]["close"] = 150
    changed_data["candles"][1]["high"] = 151
    _, changed_output = _run_target(
        implementation,
        workspace,
        run_root,
        changed_data,
        timeframe=baseline_timeframe,
        suffix=f"{target_id}-sensitivity",
        environment=environment,
    )
    if changed_output is None or changed_output == baseline:
        return failure("eligible-candle sensitivity failed")
    checks.append(f"{target_id}: eligible-candle sensitivity")

    mismatch, _ = _run_target(
        implementation,
        workspace,
        run_root,
        baseline_data,
        symbol="ETH",
        timeframe=baseline_timeframe,
        suffix=f"{target_id}-context-mismatch",
        environment=environment,
    )
    if mismatch.returncode == 0:
        return failure("CLI/input context mismatch was accepted")
    checks.append(f"{target_id}: CLI/input context mismatch rejection")

    for timeframe in declared_timeframes:
        interval_ms = INTERVAL_MS[timeframe]
        interval_fixture = _fixture_for_interval(timeframe, interval_ms)
        interval_run, interval_output = _run_target(
            implementation,
            workspace,
            run_root,
            interval_fixture,
            suffix=f"{target_id}-interval-{timeframe}",
            environment=environment,
        )
        error = _validate_output(
            interval_output,
            timeframe=timeframe,
            expected_count=2,
        )
        if interval_run.returncode != 0 or error is not None:
            return failure(
                f"declared timeframe {timeframe} failed: {error or 'execution failed'}",
                interval_run,
            )
    checks.append(f"{target_id}: declared timeframes")
    return None


def validate(workspace: Path) -> dict[str, object]:
    user = (workspace / "scripts" / "user").resolve()
    if not user.is_dir():
        return {"ok": False, "checks": ["scripts/user is missing"]}

    checks: list[str] = []
    targets, manifest_error = _validate_manifest(user)
    fixture = Path(__file__).with_name("coding_fixture.json")
    if manifest_error is not None or not targets or not fixture.is_file():
        return _failure(checks, manifest_error or "analysis fixture is missing")

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
            checks.append("package manifest")

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

            for target_id, implementation, declared_timeframes in targets:
                failure = _validate_target_suite(
                    checks,
                    target_id,
                    implementation,
                    workspace,
                    run_root,
                    fixture_data,
                    declared_timeframes,
                    environment,
                )
                if failure is not None:
                    return failure
    except (OSError, ValueError, json.JSONDecodeError, subprocess.SubprocessError) as error:
        return _failure(checks, f"coding validation error: {error}")
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

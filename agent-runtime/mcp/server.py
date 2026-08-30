"""HyperVibes MCP server.

Runtime infrastructure that exposes HyperVibes backend endpoints to
OpenCode agents as MCP tools. See ``README.md`` for context.

The server:

* Loads ``.env`` from the current working directory (the agent workspace)
  defensively. Required vars: ``HYPERVIBES_API_BASE_URL``,
  ``HYPERVIBES_API_KEY``, ``HYPERVIBES_AGENT_KEY``.
* Never prints secret values.
* Returns JSON-compatible dict/list values only.
* Maps backend HTTP errors to MCP errors without leaking the bearer token.
"""

from __future__ import annotations

import hashlib
import json
import logging
import os
import secrets
import subprocess
import sys
from pathlib import Path
from typing import Any

import httpx
from dotenv import load_dotenv
from mcp.server.fastmcp import FastMCP


HTTP_TIMEOUT_SECONDS = 30.0

mcp = FastMCP("hypervibes")


# MCP uses stdout for its JSON-RPC transport. Keep operational diagnostics on
# stderr so they cannot corrupt tool responses.
LOGGER = logging.getLogger("hypervibes.mcp")
LOGGER.setLevel(logging.INFO)
if not LOGGER.handlers:
    formatter = logging.Formatter("%(asctime)s %(levelname)s %(message)s")
    stderr_handler = logging.StreamHandler(sys.stderr)
    stderr_handler.setFormatter(formatter)
    LOGGER.addHandler(stderr_handler)
LOGGER.propagate = False


def _redirect_stderr_to_container_log() -> None:
    """Bypass OpenCode's MCP stderr capture without touching stdio stdout."""
    try:
        container_stdout = os.open("/proc/1/fd/1", os.O_WRONLY)
        try:
            os.dup2(container_stdout, sys.stderr.fileno())
        finally:
            os.close(container_stdout)
    except OSError:
        # This path is unavailable outside the container, such as unit tests.
        pass


def _load_config() -> tuple[str, str, str]:
    """Read required env vars, failing fast with a clear message.

    Returns ``(base_url, api_key, agent_key)``. ``base_url`` has any
    trailing slash stripped.
    """
    load_dotenv(dotenv_path=Path.cwd() / ".env")
    base_url = os.getenv("HYPERVIBES_API_BASE_URL", "").strip().rstrip("/")
    api_key = os.getenv("HYPERVIBES_API_KEY", "").strip()
    agent_key = os.getenv("HYPERVIBES_AGENT_KEY", "").strip()
    missing = [
        name
        for name, value in (
            ("HYPERVIBES_API_BASE_URL", base_url),
            ("HYPERVIBES_API_KEY", api_key),
            ("HYPERVIBES_AGENT_KEY", agent_key),
        )
        if not value
    ]
    if missing:
        joined = ", ".join(missing)
        raise RuntimeError(
            "HyperVibes MCP server is missing required environment "
            f"variables: {joined}"
        )
    return base_url, api_key, agent_key


CONFIG: tuple[str, str, str] = _load_config()


def _require_config() -> tuple[str, str, str]:
    """Return the cached config, reloading on first call after failure."""
    global CONFIG
    base_url, api_key, agent_key = CONFIG
    if not base_url or not api_key or not agent_key:
        CONFIG = _load_config()
        base_url, api_key, agent_key = CONFIG
    return CONFIG


def _headers() -> dict[str, str]:
    _, api_key, _ = _require_config()
    return {"Authorization": f"Bearer {api_key}"}


def _request(
    method: str,
    path: str,
    *,
    params: dict[str, Any] | None = None,
    json_body: Any = None,
) -> Any:
    """Make an authenticated request to the HyperVibes backend.

    Translates HTTP errors into ``RuntimeError`` with a redacted message.
    """
    base_url, _, _ = _require_config()
    url = f"{base_url}{path}"
    try:
        response = httpx.request(
            method,
            url,
            params=params,
            json=json_body,
            headers=_headers(),
            timeout=HTTP_TIMEOUT_SECONDS,
        )
    except httpx.HTTPError as exc:
        raise RuntimeError(f"HyperVibes request failed: {exc}") from exc

    if response.status_code >= 400:
        detail = response.text.strip()
        # Do not log the raw body: a misconfigured backend could echo
        # credentials or memory content in it. Preserve the existing redacted
        # error for the tool caller, which is useful for actionable API errors.
        detail = detail.replace(_require_config()[1], "[redacted]")
        LOGGER.warning(
            "hypervibes_mcp_request_failed method=%s path=%s params=%r status=%s",
            method,
            path,
            params or {},
            response.status_code,
        )
        raise RuntimeError(
            f"HyperVibes {method} {path} returned "
            f"{response.status_code}: {detail[:500]}"
        )

    if not response.content:
        LOGGER.info(
            "hypervibes_mcp_request method=%s path=%s params=%r status=%s response=empty",
            method,
            path,
            params or {},
            response.status_code,
        )
        return None
    try:
        result = response.json()
    except ValueError as exc:
        raise RuntimeError(
            f"HyperVibes {method} {path} returned non-JSON body"
        ) from exc
    if isinstance(result, list):
        response_shape = f"list:{len(result)}"
    elif isinstance(result, dict):
        response_shape = "object"
    else:
        response_shape = type(result).__name__
    LOGGER.info(
        "hypervibes_mcp_request method=%s path=%s params=%r status=%s response=%s",
        method,
        path,
        params or {},
        response.status_code,
        response_shape,
    )
    return result


def _require_nonblank(name: str, value: str) -> str:
    if not value or not value.strip():
        raise ValueError(f"{name} must not be blank")
    return value


STRATEGY_PROMPT_KINDS = {
    "analysis",
    "market_analysis",
    "trading",
    "daily_review",
    "analysis_coding",
}


def _require_strategy_prompt_kind(prompt_kind: str) -> str:
    if not isinstance(prompt_kind, str) or prompt_kind not in STRATEGY_PROMPT_KINDS:
        allowed = ", ".join(sorted(STRATEGY_PROMPT_KINDS))
        raise ValueError(f"prompt_kind must be one of: {allowed}")
    return prompt_kind


def _require_strategy_prompt_response(value: Any) -> dict[str, Any]:
    if (
        not isinstance(value, dict)
        or set(value) != {"prompt_kind", "prompt", "updated_at"}
        or not isinstance(value["prompt_kind"], str)
        or not isinstance(value["prompt"], str)
        or not isinstance(value["updated_at"], str)
    ):
        raise RuntimeError("HyperVibes strategy prompt returned unexpected shape")
    return value


def _require_limit(limit: int | None) -> int | None:
    if limit is None:
        return None
    if limit < 1:
        raise ValueError("limit must be >= 1")
    return limit


def _require_offset(offset: int | None) -> int | None:
    if offset is None:
        return None
    if offset < 0:
        raise ValueError("offset must be >= 0")
    return offset


CODING_ALLOWED_SUFFIXES = {".py", ".json", ".md"}
CODING_MAX_FILE_BYTES = 1024 * 1024
CODING_MAX_TOTAL_BYTES = 20 * 1024 * 1024
CODING_VALIDATOR_PYTHON = "/opt/hypervibes/analysis/.venv/bin/python"
CODING_VALIDATOR_SCRIPT = "/opt/hypervibes/coding/coding_validate.py"
CODING_VALIDATOR_TIMEOUT_SECONDS = 65
ANALYSIS_RUNTIME_PYTHON = "/opt/hypervibes/analysis/.venv/bin/python"
ANALYSIS_TOOL_TIMEOUT_SECONDS = 30
ANALYSIS_TOOL_MANIFEST = "manifest.json"
ANALYSIS_TOOL_REQUIRED_ARGUMENTS = {
    "symbol",
    "timeframe",
    "boundary_ms",
    "input",
    "output",
}


def _coding_user_root() -> Path:
    root = (Path.cwd() / "scripts" / "user").resolve()
    root.mkdir(parents=True, exist_ok=True)
    return root


def _coding_task_id() -> int:
    value = os.getenv("HYPERVIBES_CODING_TASK_ID", "").strip()
    try:
        task_id = int(value)
    except ValueError as exc:
        raise RuntimeError("coding task id is missing or invalid") from exc
    if task_id <= 0:
        raise RuntimeError("coding task id must be positive")
    return task_id


def _coding_hash(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _coding_manifest_hash() -> str:
    root = _coding_user_root()
    files: list[tuple[str, str]] = []
    total = 0
    for path in sorted(root.rglob("*")):
        if path.name == "__pycache__" or path.name.endswith((".pyc", "~")):
            continue
        if path.is_symlink():
            raise RuntimeError("candidate symlinks are not allowed")
        if not path.is_file():
            continue
        if path.suffix.lower() not in CODING_ALLOWED_SUFFIXES:
            raise RuntimeError(f"candidate file extension is not allowed: {path.name}")
        size = path.stat().st_size
        if size > CODING_MAX_FILE_BYTES:
            raise RuntimeError("candidate file exceeds size limit")
        total += size
        if total > CODING_MAX_TOTAL_BYTES:
            raise RuntimeError("candidate tree exceeds size limit")
        files.append((path.relative_to(root).as_posix(), _coding_hash(path)))
    digest = hashlib.sha256()
    for relative, file_hash in files:
        digest.update(relative.encode())
        digest.update(b"\0")
        digest.update(file_hash.encode())
        digest.update(b"\0")
    return digest.hexdigest()


def _coding_validation_path() -> Path:
    return Path.cwd().resolve().parent / "coding-validation.json"


def _analysis_user_root() -> Path:
    root = (Path.cwd() / "scripts" / "user").resolve()
    if not root.is_dir() or root.parent.parent != Path.cwd().resolve():
        raise RuntimeError("analysis package root is unavailable")
    return root


def _analysis_manifest() -> tuple[dict[str, Any], Path]:
    root = _analysis_user_root()
    path = root / ANALYSIS_TOOL_MANIFEST
    try:
        manifest = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        raise RuntimeError("analysis-tool manifest is missing or invalid") from exc
    if (
        not isinstance(manifest, dict)
        or manifest.get("schema_version") != 1
        or not isinstance(manifest.get("package_version"), str)
        or not manifest["package_version"].strip()
        or not isinstance(manifest.get("tools"), list)
    ):
        raise RuntimeError("analysis-tool manifest has an invalid schema")
    return manifest, root


def _analysis_package_hash(root: Path) -> str:
    return _coding_manifest_hash_for_root(root)


def _coding_manifest_hash_for_root(root: Path) -> str:
    files: list[tuple[str, str]] = []
    for path in sorted(root.rglob("*")):
        if path.name == "__pycache__" or path.name.endswith((".pyc", "~")):
            continue
        if path.is_symlink():
            raise RuntimeError("analysis package symlinks are not allowed")
        if path.is_file():
            files.append((path.relative_to(root).as_posix(), _coding_hash(path)))
    digest = hashlib.sha256()
    for relative, file_hash in files:
        digest.update(relative.encode())
        digest.update(b"\0")
        digest.update(file_hash.encode())
        digest.update(b"\0")
    return digest.hexdigest()


def _analysis_scratch_path(value: str, name: str) -> Path:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{name} must be a nonblank scratch-relative path")
    workspace = Path.cwd().resolve()
    scratch = (workspace / "scratch").resolve()
    path = (workspace / value).resolve()
    if scratch.parent != workspace or not path.is_relative_to(scratch):
        raise ValueError(f"{name} must remain under scratch/")
    return path


def _validate_analysis_output(
    output: Any, symbol: str, timeframe: str, boundary_ms: int
) -> None:
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
        raise RuntimeError("analysis tool returned an invalid output envelope")
    if output["symbol"] != symbol or output["timeframe"] != timeframe:
        raise RuntimeError("analysis tool output context does not match requested tool input")
    if output["boundary_ms"] != boundary_ms:
        raise RuntimeError("analysis tool output boundary does not match requested tool input")
    if not isinstance(output["source_range"], dict) or not isinstance(
        output["source_range"].get("count"), int
    ):
        raise RuntimeError("analysis tool output source range is invalid")
    if not isinstance(output["measurements"], dict) or not output["measurements"]:
        raise RuntimeError("analysis tool output measurements are invalid")
    if not isinstance(output["warnings"], list):
        raise RuntimeError("analysis tool output warnings are invalid")


@mcp.tool()
def run_analysis_tool(
    tool_id: str,
    symbol: str,
    timeframe: str,
    boundary_ms: int,
    input_path: str,
    output_path: str,
) -> dict[str, Any]:
    """Run one manifest-declared quantitative tool in the current workspace.

    Inputs and outputs must be run-local paths beneath scratch/. The tool result
    is validated and bound to the exact package version and content hash.
    """
    manifest, root = _analysis_manifest()
    tool = next(
        (item for item in manifest["tools"] if isinstance(item, dict) and item.get("id") == tool_id),
        None,
    )
    if tool is None:
        raise ValueError("analysis tool is not declared by the package manifest")
    if (
        not isinstance(tool.get("description"), str)
        or not tool["description"].strip()
        or not isinstance(tool.get("entrypoint"), str)
        or Path(tool["entrypoint"]).is_absolute()
        or ".." in Path(tool["entrypoint"]).parts
        or not isinstance(tool.get("version"), str)
        or not tool["version"].strip()
        or not isinstance(tool.get("required_arguments"), list)
        or set(tool["required_arguments"]) != ANALYSIS_TOOL_REQUIRED_ARGUMENTS
        or tool.get("input_kind") != "ohlcv"
        or tool.get("output_schema") != "hypervibes.quantitative.v1"
        or not isinstance(tool.get("supported_timeframes"), list)
        or timeframe not in tool.get("supported_timeframes", [])
        or not isinstance(tool.get("minimum_candles"), int)
        or tool["minimum_candles"] < 1
    ):
        raise RuntimeError("analysis tool declaration is invalid")
    entrypoint = (root / tool["entrypoint"]).resolve()
    if not entrypoint.is_relative_to(root) or not entrypoint.is_file():
        raise RuntimeError("analysis tool entrypoint is unavailable")
    input_file = _analysis_scratch_path(input_path, "input_path")
    output_file = _analysis_scratch_path(output_path, "output_path")
    if not input_file.is_file():
        raise ValueError("input_path does not name an existing regular file")
    output_file.parent.mkdir(parents=True, exist_ok=True)
    environment = {
        "HOME": str((Path.cwd() / "scratch").resolve()),
        "TMPDIR": str((Path.cwd() / "scratch" / "tmp").resolve()),
        "PYTHONDONTWRITEBYTECODE": "1",
        "PYTHONHASHSEED": "0",
        "PATH": str(Path(ANALYSIS_RUNTIME_PYTHON).parent),
    }
    try:
        completed = subprocess.run(
            [
                ANALYSIS_RUNTIME_PYTHON, str(entrypoint), "--symbol", symbol,
                "--timeframe", timeframe, "--boundary-ms", str(boundary_ms),
                "--input", str(input_file), "--output", str(output_file),
            ],
            cwd=Path.cwd(), capture_output=True, text=True,
            timeout=ANALYSIS_TOOL_TIMEOUT_SECONDS, env=environment,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise RuntimeError(f"analysis tool could not run: {exc}") from exc
    if completed.returncode != 0 or not output_file.is_file():
        raise RuntimeError("analysis tool failed to produce output")
    try:
        output = json.loads(output_file.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        raise RuntimeError("analysis tool produced invalid JSON") from exc
    _validate_analysis_output(output, symbol, timeframe, boundary_ms)
    binding = {
        "package_version": manifest["package_version"],
        "package_manifest_hash": _analysis_package_hash(root),
        "tool_id": tool_id,
        "tool_version": tool["version"],
    }
    output["quantitative_package"] = binding
    temporary = output_file.with_suffix(output_file.suffix + ".tmp")
    temporary.write_text(json.dumps(output, sort_keys=True) + "\n")
    os.replace(temporary, output_file)
    return {"output_path": output_path, **binding}


def _invalidate_coding_validation() -> None:
    _coding_validation_path().unlink(missing_ok=True)


@mcp.tool()
def coding_validate_candidate() -> dict[str, Any]:
    """Run the fixed validator and bind its result to the candidate tree."""
    task_id = _coding_task_id()
    workspace = Path.cwd().resolve()
    _invalidate_coding_validation()
    before = _coding_manifest_hash()
    environment = {
        "HOME": "/tmp",
        "PATH": str(Path(CODING_VALIDATOR_PYTHON).parent),
        "PYTHONHASHSEED": "0",
    }
    try:
        completed = subprocess.run(
            [
                CODING_VALIDATOR_PYTHON,
                CODING_VALIDATOR_SCRIPT,
                "--workspace",
                str(workspace),
            ],
            cwd=workspace,
            capture_output=True,
            text=True,
            timeout=CODING_VALIDATOR_TIMEOUT_SECONDS,
            env=environment,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise RuntimeError(f"fixed coding validator could not run: {exc}") from exc
    try:
        result = json.loads(completed.stdout.strip())
    except json.JSONDecodeError as exc:
        raise RuntimeError("fixed coding validator returned invalid JSON") from exc
    if not isinstance(result, dict) or not isinstance(result.get("ok"), bool):
        raise RuntimeError("fixed coding validator returned unexpected output")
    after = _coding_manifest_hash()
    if before != after:
        result = {
            "ok": False,
            "checks": list(result.get("checks", [])) + ["candidate changed during validation"],
        }
    if completed.returncode == 0 and not result["ok"]:
        raise RuntimeError("fixed coding validator status disagrees with its report")
    if completed.returncode != 0 and result["ok"]:
        raise RuntimeError("fixed coding validator status disagrees with its report")
    validation = {
        **result,
        "schema_version": 1,
        "task_id": task_id,
        "validation_id": secrets.token_hex(16),
        "candidate_manifest_sha256": after,
    }
    path = _coding_validation_path()
    temporary = path.with_suffix(".json.tmp")
    temporary.write_text(json.dumps(validation, sort_keys=True, indent=2) + "\n")
    os.replace(temporary, path)
    return validation


@mcp.tool()
def coding_submit_report(
    outcome: str,
    summary: str,
    rationale: str,
    changed_paths: list[str],
    evidence_memory_ids: list[str],
    validation_notes: str,
) -> dict[str, Any]:
    """Submit one report after validation; paths are relative to scripts/user."""
    if outcome not in {"changed", "no_change"}:
        raise ValueError("outcome must be changed or no_change")
    if any(
        not isinstance(path, str)
        or path.startswith("/")
        or ".." in Path(path).parts
        or Path(path).parts[:2] == ("scripts", "user")
        for path in changed_paths
    ):
        raise ValueError(
            "changed paths must be relative to scripts/user "
            "(for example analyze.py, not scripts/user/analyze.py)"
        )
    result = _request(
        "POST",
        "/api/v1/coding/report",
        json_body={
            "task_id": _coding_task_id(),
            "schema_version": 1,
            "outcome": outcome,
            "summary": summary,
            "rationale": rationale,
            "changed_paths": changed_paths,
            "evidence_memory_ids": evidence_memory_ids,
            "validation_notes": validation_notes,
        },
    )
    return result if isinstance(result, dict) else {"submitted": True}


@mcp.tool()
def get_account() -> dict[str, Any]:
    """Return this agent's current Hyperliquid account snapshot."""
    result = _request("GET", "/api/v1/account")
    if not isinstance(result, dict):
        raise RuntimeError("HyperVibes /account returned unexpected shape")
    return result


@mcp.tool()
def list_strategy_prompts() -> list[dict[str, Any]]:
    """List this agent's editable strategy prompts for chat review."""
    result = _request("GET", "/api/v1/strategy-prompts")
    if not isinstance(result, list):
        raise RuntimeError("HyperVibes strategy prompts returned unexpected shape")
    return [_require_strategy_prompt_response(prompt) for prompt in result]


@mcp.tool()
def get_strategy_prompt(prompt_kind: str) -> dict[str, Any]:
    """Get one editable strategy prompt for chat review."""
    prompt_kind = _require_strategy_prompt_kind(prompt_kind)
    result = _request("GET", f"/api/v1/strategy-prompts/{prompt_kind}")
    return _require_strategy_prompt_response(result)


@mcp.tool()
def update_strategy_prompt(prompt_kind: str, prompt: str) -> dict[str, Any]:
    """Update one strategy prompt from chat; blank prompts are allowed."""
    prompt_kind = _require_strategy_prompt_kind(prompt_kind)
    if not isinstance(prompt, str):
        raise ValueError("prompt must be a string")
    result = _request(
        "PUT",
        f"/api/v1/strategy-prompts/{prompt_kind}",
        json_body={"prompt": prompt},
    )
    return _require_strategy_prompt_response(result)


@mcp.tool()
def get_latest_analysis(symbol: str, limit: int | None = None) -> list[dict[str, Any]]:
    """Return the latest analysis memory rows for ``symbol``.

    ``limit`` is optional; when provided it must be ``>= 1``. The
    ``memory_type`` filter is fixed to ``"analysis"`` on the server.
    """
    symbol = _require_nonblank("symbol", symbol)
    _require_limit(limit)
    params: dict[str, Any] = {"symbol": symbol, "memory_type": "analysis"}
    if limit is not None:
        params["limit"] = limit
    result = _request("GET", "/api/v1/memories/latest", params=params)
    if not isinstance(result, list):
        raise RuntimeError(
            "HyperVibes /memories/latest returned unexpected shape"
        )
    return result


@mcp.tool()
def get_market_analysis(symbol: str) -> dict[str, Any] | None:
    """Return the latest fresh market-analysis memory row for ``symbol``."""
    symbol = _require_nonblank("symbol", symbol)
    result = _request(
        "GET",
        "/api/v1/memories",
        params={"symbol": symbol, "memory_type": "market_analysis", "limit": 1},
    )
    if not isinstance(result, list):
        raise RuntimeError("HyperVibes /memories returned unexpected shape")
    if not result:
        LOGGER.info("hypervibes_mcp_market_analysis symbol=%s found=false", symbol)
        return None
    first = result[0]
    if not isinstance(first, dict):
        raise RuntimeError("HyperVibes /memories returned unexpected shape")
    LOGGER.info("hypervibes_mcp_market_analysis symbol=%s found=true", symbol)
    return first


@mcp.tool()
def get_memory_detail(memory_id: str) -> dict[str, Any]:
    """Return one agent-owned memory and its links."""
    memory_id = _require_nonblank("memory_id", memory_id)
    result = _request(
        "GET",
        f"/api/v1/memories/{memory_id}",
        params={"include": "links"},
    )
    if not isinstance(result, dict):
        raise RuntimeError("HyperVibes memory detail returned unexpected shape")
    return result


@mcp.tool()
def list_memories(
    symbol: str | None = None,
    timeframe: str | None = None,
    memory_type: str | None = None,
    since: str | None = None,
    until: str | None = None,
    limit: int | None = None,
    include_expired: bool = False,
) -> list[dict[str, Any]]:
    """List memory rows visible to this agent.

    All filters are optional. ``include_expired`` defaults to ``False`` to
    match the backend's default staleness hiding.

    Use ``memory_type="daily_review"`` and ``symbol="__agent__"`` for an
    agent's daily reviews; set ``include_expired=True`` for historical reviews.
    ``agent_learnings`` is a separate durable learning-memory type.
    """
    params: dict[str, Any] = {}
    if symbol is not None:
        params["symbol"] = _require_nonblank("symbol", symbol)
    if timeframe is not None:
        params["timeframe"] = _require_nonblank("timeframe", timeframe)
    if memory_type is not None:
        params["memory_type"] = _require_nonblank("memory_type", memory_type)
    if since is not None:
        params["since"] = _require_nonblank("since", since)
    if until is not None:
        params["until"] = _require_nonblank("until", until)
    if limit is not None:
        params["limit"] = _require_limit(limit)
    if include_expired:
        params["include_expired"] = "true"
    result = _request("GET", "/api/v1/memories", params=params or None)
    if not isinstance(result, list):
        raise RuntimeError("HyperVibes /memories returned unexpected shape")
    return result


@mcp.tool()
def list_orders(
    status: str | None = None,
    symbol: str | None = None,
    since: str | None = None,
    until: str | None = None,
    limit: int | None = None,
) -> list[dict[str, Any]]:
    """List orders visible to this agent.

    Both filters are optional. When provided, ``status`` and ``symbol`` are
    forwarded to the backend unchanged.
    """
    params: dict[str, Any] = {}
    if status is not None:
        params["status"] = _require_nonblank("status", status)
    if symbol is not None:
        params["symbol"] = _require_nonblank("symbol", symbol)
    if since is not None:
        params["since"] = _require_nonblank("since", since)
    if until is not None:
        params["until"] = _require_nonblank("until", until)
    if limit is not None:
        params["limit"] = _require_limit(limit)
    result = _request("GET", "/api/v1/orders", params=params or None)
    if not isinstance(result, list):
        raise RuntimeError("HyperVibes /orders returned unexpected shape")
    return result


@mcp.tool()
def list_account_transactions(
    since: str,
    until: str,
    symbol: str | None = None,
    event_category: str | None = None,
    limit: int | None = None,
    offset: int | None = None,
) -> list[dict[str, Any]]:
    """List durable Hyperliquid fills, funding, and ledger events for this agent.

    ``since`` and ``until`` are required RFC 3339 bounds. Results come from
    HyperVibes's account journal, not a direct exchange request. Use
    ``offset`` with a fixed ``limit`` to page through a review window until a
    page returns fewer rows than the requested limit.
    """
    params: dict[str, Any] = {
        "since": _require_nonblank("since", since),
        "until": _require_nonblank("until", until),
    }
    if symbol is not None:
        params["symbol"] = _require_nonblank("symbol", symbol)
    if event_category is not None:
        params["event_category"] = _require_nonblank(
            "event_category", event_category
        )
    if limit is not None:
        params["limit"] = _require_limit(limit)
    if offset is not None:
        params["offset"] = _require_offset(offset)
    result = _request("GET", "/api/v1/account/transactions", params=params)
    if not isinstance(result, list):
        raise RuntimeError(
            "HyperVibes /account/transactions returned unexpected shape"
        )
    return result


@mcp.tool()
def get_order(order_id: str, include_events: bool = False) -> dict[str, Any]:
    """Fetch a single order by id.

    Set ``include_events=True`` to also return the order's event log.
    """
    order_id = _require_nonblank("order_id", order_id)
    params: dict[str, Any] | None = (
        {"include": "events"} if include_events else None
    )
    result = _request("GET", f"/api/v1/orders/{order_id}", params=params)
    if not isinstance(result, dict):
        raise RuntimeError("HyperVibes /orders/{id} returned unexpected shape")
    return result


@mcp.tool()
def write_memory(
    symbol: str,
    memory_type: str,
    summary: str,
    content: str,
    timeframe: str | None = None,
    metadata: dict[str, Any] | None = None,
    links: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    """Persist a memory for this agent.

    Required: ``symbol``, ``memory_type``, ``summary``, ``content``.
    ``timeframe`` is optional. For a general memory, omit the ``timeframe``
    argument entirely; do not pass an empty string. ``metadata`` must be a
    JSON object when provided; ``None`` is stored as an empty object. ``links``
    may be a list of objects with ``target_memory_id``, ``link_type``, and an
    optional object ``metadata``.
    """
    symbol = _require_nonblank("symbol", symbol)
    memory_type = _require_nonblank("memory_type", memory_type)
    summary = _require_nonblank("summary", summary)
    content = _require_nonblank("content", content)
    if timeframe is not None:
        timeframe = _require_nonblank("timeframe", timeframe)
    if metadata is not None and not isinstance(metadata, dict):
        raise ValueError("metadata must be a JSON object")
    if links is not None:
        if not isinstance(links, list):
            raise ValueError("links must be a list")
        for index, link in enumerate(links):
            if not isinstance(link, dict):
                raise ValueError(f"links[{index}] must be a JSON object")
    body: dict[str, Any] = {
        "symbol": symbol,
        "memory_type": memory_type,
        "summary": summary,
        "content": content,
    }
    if timeframe is not None:
        body["timeframe"] = timeframe
    body["metadata"] = metadata if metadata is not None else {}
    if links is not None:
        body["links"] = links
    result = _request("POST", "/api/v1/memories", json_body=body)
    if not isinstance(result, dict):
        raise RuntimeError("HyperVibes /memories POST returned unexpected shape")
    return result


@mcp.tool()
def request_analysis_coding(reason: str, mode: str = "auto") -> dict[str, Any]:
    """Queue an on-demand analysis-coding sub-agent run.

    This returns after the durable task is queued; it does not wait for model
    execution, validation, or candidate promotion. ``reason`` must explain the
    requested reusable analysis-code improvement. ``mode`` is ``auto``,
    ``bootstrap``, or ``manual_improvement``.
    """
    reason = _require_nonblank("reason", reason)
    if len(reason) > 4_000:
        raise ValueError("reason must be at most 4000 characters")
    if mode not in {"auto", "bootstrap", "manual_improvement"}:
        raise ValueError("mode must be auto, bootstrap, or manual_improvement")
    result = _request(
        "POST",
        "/api/v1/coding/requests",
        json_body={"reason": reason, "mode": mode},
    )
    if (
        not isinstance(result, dict)
        or set(result) != {"task_id", "run_id", "status"}
        or not isinstance(result["task_id"], int)
        or not isinstance(result["run_id"], int)
        or result["status"] != "queued"
    ):
        raise RuntimeError("HyperVibes coding request returned unexpected shape")
    return result


@mcp.tool()
def submit_orders(orders: list[dict[str, Any]]) -> dict[str, Any]:
    """Submit one or more orders through the HyperVibes backend.

    ``orders`` is the same list shape the backend expects on
    ``POST /api/v1/orders``. Opening agent orders should include the selected
    fresh market-analysis memory ID in ``memory_record_ids``. This is a real
    backend action: the server selects instruments, signs the request, and
    submits to Hyperliquid. The MCP server does not hold or use any private key.
    """
    if not isinstance(orders, list) or not orders:
        raise ValueError("orders must be a non-empty list")
    for index, order in enumerate(orders):
        if not isinstance(order, dict):
            raise ValueError(f"orders[{index}] must be a JSON object")
    result = _request("POST", "/api/v1/orders", json_body={"orders": orders})
    if not isinstance(result, dict):
        raise RuntimeError("HyperVibes /orders POST returned unexpected shape")
    return result


@mcp.tool()
def cancel_orders(orders: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Cancel one or more orders through the HyperVibes backend.

    ``orders`` is the same list shape the backend expects on
    ``POST /api/v1/orders/cancel``. Returns one outcome per requested
    cancel.
    """
    if not isinstance(orders, list) or not orders:
        raise ValueError("orders must be a non-empty list")
    for index, order in enumerate(orders):
        if not isinstance(order, dict):
            raise ValueError(f"orders[{index}] must be a JSON object")
    result = _request("POST", "/api/v1/orders/cancel", json_body={"orders": orders})
    if not isinstance(result, list):
        raise RuntimeError(
            "HyperVibes /orders/cancel returned unexpected shape"
        )
    return result


@mcp.tool()
def cancel_all_orders(symbol: str | None = None) -> dict[str, Any]:
    """Cancel all open orders, optionally restricted to a single symbol.

    Real backend action. The MCP server does not hold or use any
    private key.
    """
    params: dict[str, Any] | None = None
    if symbol is not None:
        symbol = _require_nonblank("symbol", symbol)
        params = {"symbol": symbol}
    result = _request("POST", "/api/v1/orders/cancel-all", params=params)
    if not isinstance(result, dict):
        raise RuntimeError(
            "HyperVibes /orders/cancel-all returned unexpected shape"
        )
    return result


NOTIFICATION_SEVERITIES = {"info", "warning", "error"}


def _require_severity(severity: str) -> str:
    if not isinstance(severity, str) or severity not in NOTIFICATION_SEVERITIES:
        allowed = ", ".join(sorted(NOTIFICATION_SEVERITIES))
        raise ValueError(f"severity must be one of: {allowed}")
    return severity


@mcp.tool()
def send_notification(title: str, body: str, severity: str = "info") -> dict[str, Any]:
    """Send a notification through the agent's configured messaging gateway.

    Args:
        title: Short notification title.
        body: Detailed notification body (plain text).
        severity: One of "info", "warning", "error". Affects gateway
            formatting (for example, a warning emoji prefix on Telegram).
    """
    title = _require_nonblank("title", title)
    body = _require_nonblank("body", body)
    severity = _require_severity(severity)
    result = _request(
        "POST",
        "/api/v1/notifications",
        json_body={
            "title": title,
            "body": body,
            "severity": severity,
        },
    )
    if not isinstance(result, dict) or not isinstance(result.get("id"), str):
        raise RuntimeError("HyperVibes /notifications returned unexpected shape")
    return result


def main() -> None:
    # Fail fast on missing config so OpenCode gets a clear stderr message
    # instead of a half-initialised server.
    _redirect_stderr_to_container_log()
    _, _, agent_key = _require_config()
    LOGGER.info("hypervibes_mcp_started agent_key=%s", agent_key)
    mcp.run()


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:  # noqa: BLE001
        print(f"hypervibes MCP server failed to start: {exc}", file=sys.stderr)
        raise

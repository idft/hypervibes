from __future__ import annotations

import os
from typing import Any

import requests


class VibetradingClient:
    def __init__(self, base_url: str | None = None, api_key: str | None = None, timeout: float = 30.0):
        self.base_url = (base_url or os.environ.get("VIBETRADING_BASE_URL", "")).rstrip("/")
        self.api_key = api_key or os.environ.get("VIBETRADING_API_KEY", "")
        self.timeout = timeout

        if not self.base_url:
            raise RuntimeError("VIBETRADING_BASE_URL is required")
        if not self.api_key:
            raise RuntimeError("VIBETRADING_API_KEY is required")

        self.session = requests.Session()
        self.session.headers.update(
            {
                "Authorization": f"Bearer {self.api_key}",
                "Accept": "application/json",
            }
        )

    def _request(self, method: str, path: str, *, params: dict[str, Any] | None = None, json_body: Any | None = None) -> Any:
        url = f"{self.base_url}{path}"
        try:
            response = self.session.request(
                method,
                url,
                params=params,
                json=json_body,
                timeout=self.timeout,
            )
        except requests.RequestException as exc:
            raise RuntimeError(f"{method} {url} failed: {exc}") from exc

        if not response.ok:
            message = response.text.strip()
            try:
                body = response.json()
                message = body.get("error", message)
            except ValueError:
                pass
            raise RuntimeError(f"{method} {url} returned {response.status_code}: {message}")

        try:
            return response.json()
        except ValueError as exc:
            raise RuntimeError(f"{method} {url} returned non-JSON response") from exc

    def get_job_context(self, job_kind: str) -> dict[str, Any]:
        return self._request("GET", "/api/v1/job-context", params={"job_kind": job_kind})

    def list_memories(
        self,
        *,
        symbol: str | None = None,
        timeframe: str | None = None,
        memory_type: str | None = None,
        since: str | None = None,
        until: str | None = None,
        limit: int | None = None,
    ) -> list[dict[str, Any]]:
        params = {
            "symbol": symbol,
            "timeframe": timeframe,
            "memory_type": memory_type,
            "since": since,
            "until": until,
            "limit": limit,
        }
        return self._request(
            "GET",
            "/api/v1/memories",
            params={k: v for k, v in params.items() if v is not None},
        )

    def write_memory(
        self,
        *,
        symbol: str,
        memory_type: str,
        summary: str,
        content: str,
        timeframe: str | None = None,
        metadata: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        body = {
            "symbol": symbol,
            "timeframe": timeframe,
            "memory_type": memory_type,
            "summary": summary,
            "content": content,
            "metadata": metadata or {},
        }
        return self._request("POST", "/api/v1/memories", json_body=body)

    def get_account(self) -> dict[str, Any]:
        return self._request("GET", "/api/v1/account")

    def list_orders(self, status: str | None = None, symbol: str | None = None) -> list[dict[str, Any]]:
        params = {"status": status, "symbol": symbol}
        return self._request(
            "GET",
            "/api/v1/orders",
            params={k: v for k, v in params.items() if v is not None},
        )

    def place_orders(self, orders: list[dict[str, Any]]) -> list[dict[str, Any]]:
        body = self._request("POST", "/api/v1/orders", json_body={"orders": orders})
        return body["results"]

    def cancel_orders(self, orders: list[dict[str, Any]]) -> list[dict[str, Any]]:
        return self._request("POST", "/api/v1/orders/cancel", json_body={"orders": orders})

    def cancel_all(self, symbol: str | None = None) -> dict[str, Any]:
        params = {"symbol": symbol} if symbol else None
        return self._request("POST", "/api/v1/orders/cancel-all", params=params)

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Any

import requests
from dotenv import load_dotenv


load_dotenv()


def _read_env(name: str) -> str:
    value = os.getenv(name, "").strip()
    if not value:
        raise RuntimeError(f"Missing required environment variable: {name}")
    return value


@dataclass
class Client:
    base_url: str
    api_key: str
    agent_key: str

    @classmethod
    def from_env(cls) -> "Client":
        return cls(
            base_url=_read_env("VIBETRADING_API_BASE_URL").rstrip("/"),
            api_key=_read_env("VIBETRADING_API_KEY"),
            agent_key=_read_env("VIBETRADING_AGENT_KEY"),
        )

    def _headers(self) -> dict[str, str]:
        return {"Authorization": f"Bearer {self.api_key}"}

    def job_context(self, job_kind: str) -> dict[str, Any]:
        response = requests.get(
            f"{self.base_url}/api/v1/job-context",
            params={"job_kind": job_kind},
            headers=self._headers(),
            timeout=30,
        )
        response.raise_for_status()
        return response.json()

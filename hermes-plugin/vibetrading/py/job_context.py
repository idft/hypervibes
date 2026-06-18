from __future__ import annotations

from .client import VibetradingClient


def get_job_context(job_kind: str) -> dict:
    return VibetradingClient().get_job_context(job_kind)

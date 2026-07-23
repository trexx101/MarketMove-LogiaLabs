"""Inference service configuration.

Loads the inference-side environment variables defined in
``deploy/config.md`` (authoritative) and ``plans/market-markov-net/REQUIREMENTS.md``.

The engine reads ``ZMQ_ENDPOINT`` and dials in. The Python side binds the
``ZMQ_BIND`` socket and serves the REQ/REP loop. They point at the same
``tcp://*:5555`` / ``tcp://inference:5555`` pair in production.

Secrets are never read here — the inference service has no Kraken access.
"""
from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path


_DEFAULTS: dict[str, str] = {
    "ZMQ_ENDPOINT": "tcp://127.0.0.1:5555",
    "ZMQ_BIND": "tcp://*:5555",
    # Wave C: legacy crypto model paths (kept for backward compat — the old
    # inference_engine.py still reads these). The equity_model.py service
    # reads TCN_PATH / LGBM_*_PATH / MODELS_DIR directly from the env.
    "MODEL_PATH": "models/qqq_tcn_v1.pt",
    "NORM_STATS_PATH": "models/norm_stats_qqq_v1.json",
}


def _get(name: str) -> str:
    value = os.environ.get(name)
    if value is None or value == "":
        return _DEFAULTS[name]
    return value


@dataclass(frozen=True)
class InferenceConfig:
    """Configuration for the ZMQ inference microservice."""

    zmq_endpoint: str
    zmq_bind: str
    model_path: Path
    norm_stats_path: Path

    @classmethod
    def from_env(cls) -> "InferenceConfig":
        model_path = Path(_get("MODEL_PATH")).expanduser().resolve()
        norm_stats_path = Path(_get("NORM_STATS_PATH")).expanduser().resolve()
        return cls(
            zmq_endpoint=_get("ZMQ_ENDPOINT"),
            zmq_bind=_get("ZMQ_BIND"),
            model_path=model_path,
            norm_stats_path=norm_stats_path,
        )

    def require_artifacts(self) -> None:
        """Fail fast if model artifacts are missing on disk."""
        missing = [p for p in (self.model_path, self.norm_stats_path) if not p.exists()]
        if missing:
            paths = ", ".join(str(p) for p in missing)
            raise FileNotFoundError(
                f"Inference artifacts not found: {paths}. "
                f"Place model.pt and norm_stats.json in /models/ "
                f"(see models/README.md)."
            )

    def summary(self) -> str:
        return (
            f"zmq_endpoint={self.zmq_endpoint} "
            f"zmq_bind={self.zmq_bind} "
            f"model_path={self.model_path} "
            f"norm_stats_path={self.norm_stats_path}"
        )

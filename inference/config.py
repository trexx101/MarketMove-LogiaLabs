"""Inference service configuration.

Loads the inference-side environment variables defined in
``deploy/config.md`` (authoritative) and ``plans/market-markov-net/REQUIREMENTS.md``.

The engine reads ``ZMQ_ENDPOINT`` and dials in. The Python side binds the
``ZMQ_BIND`` socket and serves the REQ/REP loop. They point at the same
``tcp://*:5555`` / ``tcp://inference:5555`` pair in production.

Secrets are never read here — the inference service has no exchange access.
"""
from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path


_DEFAULTS: dict[str, str] = {
    "ZMQ_ENDPOINT": "tcp://127.0.0.1:5555",
    "ZMQ_BIND": "tcp://*:5555",
    # Legacy crypto model paths — kept for backward compat with the old
    # inference_engine.py. The equity_model.py service reads TCN_PATH /
    # LGBM_*_PATH directly from the env.
    "MODEL_PATH": "models/qqq_tcn_v1.pt",
    "NORM_STATS_PATH": "models/norm_stats_qqq_v1.json",
}


# ── V1 (legacy) config ────────────────────────────────────────────────────────


@dataclass(frozen=True)
class InferenceConfig:
    """Configuration for the ZMQ inference microservice (legacy V1 protocol)."""

    zmq_endpoint: str
    zmq_bind: str
    model_path: Path
    norm_stats_path: Path

    @classmethod
    def from_env(cls) -> "InferenceConfig":
        def _get(name: str, default: str) -> str:
            v = os.environ.get(name)
            return default if (v is None or v == "") else v

        model_path = Path(_get("MODEL_PATH", _DEFAULTS["MODEL_PATH"])).expanduser().resolve()
        norm_stats_path = Path(_get("NORM_STATS_PATH", _DEFAULTS["NORM_STATS_PATH"])).expanduser().resolve()
        return cls(
            zmq_endpoint=_get("ZMQ_ENDPOINT", _DEFAULTS["ZMQ_ENDPOINT"]),
            zmq_bind=_get("ZMQ_BIND", _DEFAULTS["ZMQ_BIND"]),
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


# ── V3 equities config ─────────────────────────────────────────────────────────


@dataclass(frozen=True)
class EquityInferenceConfig:
    """Configuration for the QQQ V3 inference microservice (TCN + LightGBM ensemble)."""

    zmq_endpoint: str
    zmq_bind: str
    tcn_path: Path
    lgbm_h1_path: Path
    lgbm_h5_path: Path
    lgbm_h21_path: Path
    tcn_weight: float
    lgbm_weight: float

    @classmethod
    def from_env(cls) -> "EquityInferenceConfig":
        models_dir = Path(os.environ.get("MODELS_DIR", "models"))
        return cls(
            zmq_endpoint=os.environ.get("ZMQ_ENDPOINT", "tcp://127.0.0.1:5555"),
            zmq_bind=os.environ.get("ZMQ_BIND", "tcp://*:5555"),
            tcn_path=Path(os.environ.get("TCN_PATH", str(models_dir / "qqq_tcn_v1.pt"))),
            lgbm_h1_path=Path(os.environ.get("LGBM_H1_PATH", str(models_dir / "qqq_lgbm_h1_v1.pkl"))),
            lgbm_h5_path=Path(os.environ.get("LGBM_H5_PATH", str(models_dir / "qqq_lgbm_h5_v1.pkl"))),
            lgbm_h21_path=Path(os.environ.get("LGBM_H21_PATH", str(models_dir / "qqq_lgbm_h21_v1.pkl"))),
            tcn_weight=float(os.environ.get("TCN_WEIGHT", "0.5")),
            lgbm_weight=float(os.environ.get("LGBM_WEIGHT", "0.5")),
        )

    def require_artifacts(self) -> None:
        """Fail fast if any model artifact is missing."""
        missing = [p for p in (self.tcn_path, self.lgbm_h1_path,
                               self.lgbm_h5_path, self.lgbm_h21_path) if not p.exists()]
        if missing:
            paths = ", ".join(str(p) for p in missing)
            raise FileNotFoundError(f"V3 artifacts not found: {paths}")

    def summary(self) -> str:
        return (
            f"zmq_bind={self.zmq_bind} "
            f"tcn={self.tcn_path} "
            f"lgbm_h1={self.lgbm_h1_path} "
            f"lgbm_h5={self.lgbm_h5_path} "
            f"lgbm_h21={self.lgbm_h21_path} "
            f"weights=(tcn={self.tcn_weight} lgbm={self.lgbm_weight})"
        )

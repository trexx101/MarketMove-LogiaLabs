"""QQQ daily equities inference service (Wave C).

ZMQ REP server that loads the TCN + LightGBM ensemble and responds to V3
prediction requests with pred_1d / pred_5d / pred_21d.

V3 wire protocol:
  Request  → {"schema_version": 3, "feature_window": [[f0..f7], ...]}
  Response → {"pred_1d": float, "pred_5d": float, "pred_21d": float}

The feature_window is a sequence of 8-dim normalized feature vectors
(median/MAD normalized by the Rust engine before sending). The TCN consumes
the full sequence; the LightGBM models consume only the last timestep.
The ensemble blends them with configurable weights.
"""
from __future__ import annotations

import json
import logging
import os
import pickle
import signal
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F


# ── Structured JSON logger ────────────────────────────────────────────────────

class _JsonFormatter(logging.Formatter):
    def format(self, record: logging.LogRecord) -> str:
        payload: dict[str, Any] = {
            "ts": datetime.now(timezone.utc).isoformat(),
            "level": record.levelname,
            "msg": record.getMessage(),
        }
        if record.exc_info:
            payload["exc"] = self.formatException(record.exc_info)
        return json.dumps(payload, separators=(",", ":"))


def _build_logger() -> logging.Logger:
    logger = logging.getLogger("equity_inference")
    logger.setLevel(logging.INFO)
    if not logger.handlers:
        handler = logging.StreamHandler(sys.stdout)
        handler.setFormatter(_JsonFormatter())
        logger.addHandler(handler)
    return logger


log = _build_logger()


# ── TCN architecture (mirrors training/train_tcn.py Wave C) ───────────────────

class CausalConv1d(nn.Conv1d):
    """Causal 1-D convolution (left-only padding, sequence length preserved).

    Subclasses nn.Conv1d directly so state_dict keys match the trained
    checkpoint (e.g. ``blocks.0.conv1.weight``, not ``blocks.0.conv1.conv.weight``).
    """

    def __init__(self, in_ch: int, out_ch: int, kernel_size: int, dilation: int) -> None:
        super().__init__(in_ch, out_ch, kernel_size, dilation=dilation, padding=0)
        self._causal_padding = (kernel_size - 1) * dilation

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return super().forward(F.pad(x, (self._causal_padding, 0)))


class ResidualBlock(nn.Module):
    def __init__(self, in_ch: int, out_ch: int, kernel_size: int, dilation: int, dropout: float) -> None:
        super().__init__()
        self.conv1 = CausalConv1d(in_ch, out_ch, kernel_size, dilation)
        self.conv2 = CausalConv1d(out_ch, out_ch, kernel_size, dilation)
        self.norm1 = nn.GroupNorm(1, out_ch)
        self.norm2 = nn.GroupNorm(1, out_ch)
        self.dropout = nn.Dropout(dropout)
        self.activation = nn.SiLU()
        self.residual = nn.Conv1d(in_ch, out_ch, 1) if in_ch != out_ch else nn.Identity()

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        residual = self.residual(x)
        out = self.conv1(x)
        out = self.norm1(out)
        out = self.activation(out)
        out = self.dropout(out)
        out = self.conv2(out)
        out = self.norm2(out)
        return self.activation(out + residual)


class QqqTCN(nn.Module):
    """TCN for QQQ daily equities prediction.

    Architecture (matches training/train_tcn.py Wave C):
    - input_proj: Linear(in_dim=8 → hidden_dim=64)
    - 7× ResidualBlock with dilations [1,2,4,8,16,32,64]
    - 3 horizon heads: Linear(64→32) → SiLU → Dropout → Linear(32→1)

    Input:  (batch, seq_len, 8)  — sequence of normalized 8-dim features
    Output: list of 3 tensors, each (batch,) — raw magnitude predictions
            for horizons 1d, 5d, 21d
    """

    def __init__(self, in_dim: int = 8, hidden_dim: int = 64, dropout: float = 0.1, n_horizons: int = 3) -> None:
        super().__init__()
        self.proj = nn.Linear(in_dim, hidden_dim)
        layers = [ResidualBlock(hidden_dim, hidden_dim, 3, d, dropout) for d in [1, 2, 4, 8, 16, 32, 64]]
        self.blocks = nn.Sequential(*layers)
        self.heads = nn.ModuleList([
            nn.Sequential(
                nn.Linear(hidden_dim, hidden_dim // 2),
                nn.SiLU(),
                nn.Dropout(dropout),
                nn.Linear(hidden_dim // 2, 1),
            )
            for _ in range(n_horizons)
        ])

    def forward(self, x: torch.Tensor) -> list[torch.Tensor]:
        # x: (batch, seq_len, in_dim) → project → (batch, seq_len, hidden_dim)
        x = self.proj(x).permute(0, 2, 1)  # → (batch, hidden_dim, seq_len)
        feat = self.blocks(x)[:, :, -1]     # last timestep → (batch, hidden_dim)
        return [head(feat).squeeze(-1) for head in self.heads]


def load_tcn(model_path: str, in_dim: int = 8, hidden_dim: int = 64, dropout: float = 0.1) -> QqqTCN:
    """Load a trained TCN from a state-dict checkpoint."""
    model = QqqTCN(in_dim=in_dim, hidden_dim=hidden_dim, dropout=dropout)
    state = torch.load(model_path, map_location="cpu", weights_only=True)
    model.load_state_dict(state)
    model.eval()
    return model


# ── LightGBM loader ───────────────────────────────────────────────────────────

def load_lgbm(pickle_path: str) -> Any:
    """Load a LightGBM model from a pickle file.

    Returns the underlying ``Booster`` object (not the sklearn wrapper) so
    prediction works regardless of sklearn/lightgbm version mismatches that
    can break ``LGBMRegressor.predict()``.
    """
    with open(pickle_path, "rb") as f:
        model = pickle.load(f)
    # Use the raw Booster for prediction — immune to sklearn API drift.
    booster = getattr(model, "booster_", None) or getattr(model, "_Booster", None)
    if booster is not None:
        return booster
    return model


# ── Ensemble ──────────────────────────────────────────────────────────────────

class EquityEnsemble:
    """TCN + LightGBM ensemble for 1d/5d/21d horizon predictions.

    The TCN consumes the full feature window (seq_len × 8).
    The LightGBM models consume only the last timestep (1 × 8).
    Predictions are blended using z-score normalization to match the
    Colab walk-forward evaluation pipeline.

    Default weights: TCN 0.5, LightGBM 0.5 (equal blend).
    These should be tuned on walk-forward OOS IC.
    """

    def __init__(
        self,
        tcn: QqqTCN,
        lgbm_h1: Any,
        lgbm_h5: Any,
        lgbm_h21: Any,
        tcn_weight: float = 0.5,
        lgbm_weight: float = 0.5,
    ) -> None:
        self.tcn = tcn
        self.lgbm_models = {1: lgbm_h1, 5: lgbm_h5, 21: lgbm_h21}
        self.tcn_weight = tcn_weight
        self.lgbm_weight = lgbm_weight
        self._horizons = [1, 5, 21]

    def predict(
        self, feature_window: list[list[float]], atr_ratio: float = 0.005
    ) -> dict[str, float]:
        """Run ensemble prediction on a normalized feature window.

        Parameters
        ----------
        feature_window : list of [f0..f7] floats, shape (seq_len, 8)
        atr_ratio : ATR(14) / close for the latest candle.
            Defaults to 0.005 (~0.5%, a reasonable QQQ long-run estimate).
            The Rust scheduler computes this and passes it in the V3 request.

        Returns
        -------
        dict with keys pred_1d, pred_5d, pred_21d in raw log-return units.

        Blending: raw weighted average of denormalized model outputs.
        Both TCN and LightGBM produce label-space values (ATR-normalized),
        so after denormalization they share the same units and a plain
        weighted average is appropriate.  The Colab notebook uses z-score
        blending with rolling statistics per horizon — that requires a
        prediction history buffer which is not maintained here.
        """
        # --- TCN path ---
        x = torch.tensor(feature_window, dtype=torch.float32).unsqueeze(0)  # (1, seq_len, 8)
        with torch.no_grad():
            tcn_out = self.tcn(x)  # list of 3 tensors, each (1,)
        tcn_preds = {h: float(t.squeeze()) for h, t in zip(self._horizons, tcn_out)}

        # --- LightGBM path (last timestep only) ---
        last_row = np.array(feature_window[-1], dtype=np.float64).reshape(1, -1)  # (1, 8)
        lgbm_preds = {}
        for h in self._horizons:
            lgbm_preds[h] = float(self.lgbm_models[h].predict(last_row)[0])

        # --- Weighted raw blend (both models now in label/ATR-normalized space) ---
        result = {}
        for h in self._horizons:
            # Blend in label space, then denormalize to raw log-return:
            #   label = w_t * tcn[h] + w_l * lgbm[h]
            #   raw  = label * atr_ratio
            label = self.tcn_weight * tcn_preds[h] + self.lgbm_weight * lgbm_preds[h]
            raw_log_return = label * atr_ratio
            result[f"pred_{h}d"] = float(raw_log_return)

        return result


# ── Request handling ──────────────────────────────────────────────────────────

def _handle_request(raw_bytes: bytes, ensemble: EquityEnsemble, req_id: int) -> bytes:
    """Decode one V3 REQ message, run inference, return serialized reply."""
    try:
        request: dict[str, Any] = json.loads(raw_bytes)
    except json.JSONDecodeError as exc:
        log.error("req_id=%d json_error=%s", req_id, str(exc))
        return json.dumps({"error": f"json decode error: {exc}"}).encode()

    feature_window = request.get("feature_window")
    if (
        not isinstance(feature_window, list)
        or not feature_window
        or not isinstance(feature_window[0], list)
    ):
        log.error("req_id=%d invalid_input", req_id)
        return json.dumps({"error": "feature_window must be a non-empty list of lists"}).encode()

    # ATR ratio for denormalization (optional, defaults to 0.005).
    atr_ratio = request.get("atr_ratio")
    if atr_ratio is None:
        atr_ratio = 0.005
    try:
        atr_ratio = float(atr_ratio)
    except (TypeError, ValueError):
        log.error("req_id=%d invalid atr_ratio=%r", req_id, atr_ratio)
        return json.dumps({"error": "atr_ratio must be a number"}).encode()

    # Validate feature dimension
    n_features = len(feature_window[0])
    if n_features != 8:
        err = f"expected 8 features per timestep, got {n_features}"
        log.error("req_id=%d %s", req_id, err)
        return json.dumps({"error": err}).encode()

    try:
        preds = ensemble.predict(feature_window, atr_ratio=atr_ratio)
    except Exception as exc:
        log.error("req_id=%d inference_error=%s", req_id, str(exc))
        return json.dumps({"error": f"inference error: {exc}"}).encode()

    # Validate outputs are finite
    for key, val in preds.items():
        if not isinstance(val, float) or not np.isfinite(val):
            err = f"{key}={val} is not finite"
            log.error("req_id=%d %s", req_id, err)
            return json.dumps({"error": err}).encode()

    log.info(
        json.dumps(
            {
                "req_id": req_id,
                "seq_len": len(feature_window),
                "n_features": n_features,
                "atr_ratio": atr_ratio,
                **preds,
            },
            separators=(",", ":"),
        )
    )

    return json.dumps(preds).encode()


# ── Service entry point ───────────────────────────────────────────────────────

def _wait_for_artifact(path: str, max_wait: int = 60) -> None:
    """Block until path exists (useful for Docker startup ordering)."""
    deadline = time.monotonic() + max_wait
    p = Path(path)
    while not p.exists():
        if time.monotonic() >= deadline:
            raise FileNotFoundError(f"Artifact not found after {max_wait}s: {path}")
        log.info("waiting for artifact: %s", path)
        time.sleep(2)


def _load_ensemble(
    tcn_path: str,
    lgbm_h1_path: str,
    lgbm_h5_path: str,
    lgbm_h21_path: str,
    tcn_weight: float,
    lgbm_weight: float,
) -> EquityEnsemble:
    """Load all model artifacts and construct the ensemble."""
    log.info("loading TCN from %s", tcn_path)
    tcn = load_tcn(tcn_path)
    log.info("tcn loaded — parameters=%d", sum(p.numel() for p in tcn.parameters()))

    log.info("loading LightGBM models")
    lgbm_h1 = load_lgbm(lgbm_h1_path)
    lgbm_h5 = load_lgbm(lgbm_h5_path)
    lgbm_h21 = load_lgbm(lgbm_h21_path)
    log.info("lightgbm loaded — h1/h5/h21 boosters ready")

    return EquityEnsemble(tcn, lgbm_h1, lgbm_h5, lgbm_h21, tcn_weight, lgbm_weight)


def run_service(
    zmq_bind: str,
    tcn_path: str,
    lgbm_h1_path: str,
    lgbm_h5_path: str,
    lgbm_h21_path: str,
    tcn_weight: float = 0.5,
    lgbm_weight: float = 0.5,
) -> int:
    """Start the ZMQ REP loop. Blocks until shutdown signal."""
    try:
        import zmq
    except ImportError:
        log.error("pyzmq is not installed; cannot start inference service")
        return 1

    # ── Graceful shutdown ─────────────────────────────────────────────────────
    _stop = {"flag": False}

    def _handle_signal(signum: int, _frame: object) -> None:
        log.info("received signal %d — shutting down", signum)
        _stop["flag"] = True

    signal.signal(signal.SIGINT, _handle_signal)
    signal.signal(signal.SIGTERM, _handle_signal)

    # ── Load ensemble ─────────────────────────────────────────────────────────
    try:
        ensemble = _load_ensemble(
            tcn_path, lgbm_h1_path, lgbm_h5_path, lgbm_h21_path,
            tcn_weight, lgbm_weight,
        )
    except Exception as exc:
        log.error("failed to load ensemble: %s", exc)
        return 1

    # ── Bind ZMQ socket ───────────────────────────────────────────────────────
    context = zmq.Context()
    socket = context.socket(zmq.REP)
    try:
        socket.bind(zmq_bind)
    except zmq.ZMQError as exc:
        log.error("failed to bind ZMQ socket %s: %s", zmq_bind, exc)
        context.destroy()
        return 1

    log.info("ZMQ REP bound to %s — ready (V3 equities)", zmq_bind)

    # ── REQ/REP loop ──────────────────────────────────────────────────────────
    req_id = 0
    poller = zmq.Poller()
    poller.register(socket, zmq.POLLIN)

    while not _stop["flag"]:
        try:
            events = dict(poller.poll(timeout=500))
        except zmq.ZMQError:
            break

        if socket not in events:
            continue

        try:
            raw = socket.recv(flags=zmq.NOBLOCK)
        except zmq.Again:
            continue

        req_id += 1
        reply = _handle_request(raw, ensemble, req_id)
        socket.send(reply)

    log.info("shutting down — processed %d requests", req_id)
    socket.close()
    context.term()
    return 0


def main() -> int:
    # ── Resolve paths from env ────────────────────────────────────────────────
    models_dir = Path(os.environ.get("MODELS_DIR", "models"))

    tcn_path = os.environ.get("TCN_PATH", str(models_dir / "qqq_tcn_v1.pt"))
    lgbm_h1_path = os.environ.get("LGBM_H1_PATH", str(models_dir / "qqq_lgbm_h1_v1.pkl"))
    lgbm_h5_path = os.environ.get("LGBM_H5_PATH", str(models_dir / "qqq_lgbm_h5_v1.pkl"))
    lgbm_h21_path = os.environ.get("LGBM_H21_PATH", str(models_dir / "qqq_lgbm_h21_v1.pkl"))
    zmq_bind = os.environ.get("ZMQ_BIND", "tcp://*:5555")
    tcn_weight = float(os.environ.get("TCN_WEIGHT", "0.5"))
    lgbm_weight = float(os.environ.get("LGBM_WEIGHT", "0.5"))

    # ── Verify artifacts exist ────────────────────────────────────────────────
    for label, path in [("TCN", tcn_path), ("LGBM-h1", lgbm_h1_path),
                         ("LGBM-h5", lgbm_h5_path), ("LGBM-h21", lgbm_h21_path)]:
        if not Path(path).exists():
            print(f"error: {label} artifact not found: {path}", file=sys.stderr)
            return 1

    log.info(
        "equity inference configured: tcn=%s lgbm=[%s,%s,%s] zmq_bind=%s weights=(tcn=%.2f lgbm=%.2f)",
        tcn_path, lgbm_h1_path, lgbm_h5_path, lgbm_h21_path, zmq_bind, tcn_weight, lgbm_weight,
    )

    return run_service(zmq_bind, tcn_path, lgbm_h1_path, lgbm_h5_path, lgbm_h21_path,
                       tcn_weight, lgbm_weight)


if __name__ == "__main__":
    raise SystemExit(main())

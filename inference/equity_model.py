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
from collections import deque
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

class CausalConv1d(nn.Module):
    """Causal 1-D convolution (left-only padding, sequence length preserved).

    Mirrors the notebook's CausalConv1d which wraps nn.Conv1d as self.conv,
    producing state_dict keys like ``blocks.0.conv1.conv.weight``.
    """

    def __init__(self, in_ch: int, out_ch: int, kernel_size: int, dilation: int) -> None:
        super().__init__()
        self.conv = nn.Conv1d(in_ch, out_ch, kernel_size, dilation=dilation, padding=0)
        self._causal_padding = (kernel_size - 1) * dilation

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return self.conv(F.pad(x, (self._causal_padding, 0)))


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
        # Training (notebook) residual: no activation after add.
        # Matches: `x_pad = block(x); x = x_pad + x`
        return out + residual


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

    §8.4 z-score blending: each model maintains a rolling buffer of
    recent predictions per horizon. Before blending, each model's raw
    prediction is z-scored against its own buffer history. This removes
    per-model bias and matches the notebook's walk-forward evaluation.
    """

    BUFFER_SIZE = 252  # ~1 year of trading days

    def __init__(
        self,
        tcn: QqqTCN,
        lgbm_h1: Any,
        lgbm_h5: Any,
        lgbm_h21: Any,
        tcn_weight: float = 0.5,
        lgbm_weight: float = 0.5,
        model_meta_path: str | None = None,
    ) -> None:
        self.tcn = tcn
        self.lgbm_models = {1: lgbm_h1, 5: lgbm_h5, 21: lgbm_h21}
        self.tcn_weight = tcn_weight
        self.lgbm_weight = lgbm_weight
        self._horizons = [1, 5, 21]

        # Per-horizon prediction buffers for z-score blending.
        # Each buffer stores raw label-space predictions (before
        # denormalization) from both TCN and LGBM separately.
        self._tcn_buffer: dict[int, deque[float]] = {h: deque(maxlen=self.BUFFER_SIZE) for h in self._horizons}
        self._lgbm_buffer: dict[int, deque[float]] = {h: deque(maxlen=self.BUFFER_SIZE) for h in self._horizons}

        # Fixed training-time label std per horizon (Deferred Fix 2).
        # The live system previously used a non-stationary buffer-based pooled
        # std for de-normalization, which made the same feature window produce
        # a different raw prediction depending on call history. The notebook
        # de-normalizes using the training-time label std, which is stationary.
        # Defaults below are typical QQQ values; overridden by model_meta when
        # present (see training notebook cell 14 / walk-forward evaluation).
        self._label_std: dict[int, float] = {1: 0.012, 5: 0.028, 21: 0.065}
        if model_meta_path:
            meta_path = Path(model_meta_path)
            if meta_path.exists():
                try:
                    meta = json.loads(meta_path.read_text())
                    for h in self._horizons:
                        key = f"label_std_{h}d"
                        if key in meta:
                            self._label_std[h] = float(meta[key])
                    log.info(
                        "loaded label_std from %s: %s",
                        model_meta_path,
                        self._label_std,
                    )
                except Exception as exc:  # noqa: BLE001 — surface but never crash boot
                    log.warning(
                        "failed to parse label_std from %s: %s", model_meta_path, exc
                    )
            else:
                log.info(
                    "model_meta not found at %s — using default label_std %s",
                    model_meta_path,
                    self._label_std,
                )

    def predict(
        self, feature_window: list[list[float]], atr_ratio: float = 0.005,
        skip_buffer: bool = False,
    ) -> dict[str, float]:
        """Run ensemble prediction on a normalized feature window.

        Parameters
        ----------
        feature_window : list of [f0..f7] floats, shape (seq_len, 8)
        atr_ratio : ATR(14) / close for the latest candle.
        skip_buffer : if True, do not update z-score prediction buffers
                      (used for healthcheck pings).

        Returns
        -------
        dict with keys pred_1d, pred_5d, pred_21d in raw log-return units.

        Blending: z-score normalized weighted average matching the Colab
        walk-forward evaluation pipeline.
        """
        # --- TCN path ---
        x = torch.tensor(feature_window, dtype=torch.float32).unsqueeze(0)
        with torch.no_grad():
            tcn_out = self.tcn(x)
        tcn_preds = {h: float(t.squeeze()) for h, t in zip(self._horizons, tcn_out)}

        # --- LightGBM path (last timestep only) ---
        last_row = np.array(feature_window[-1], dtype=np.float64).reshape(1, -1)
        lgbm_preds = {h: float(self.lgbm_models[h].predict(last_row)[0]) for h in self._horizons}

        # --- Z-score blending ---
        result = {}
        for h in self._horizons:
            tcn_raw = tcn_preds[h]
            lgbm_raw = lgbm_preds[h]

            # Warmup: buffers are too short for a reliable z-score. Use the raw
            # 0.5/0.5 blend of the two model outputs (Deferred Fix 2). The raw
            # blend lives in the same label-space units as the de-normalized
            # output, so it is a reasonable stand-in until the buffer fills.
            if len(self._tcn_buffer[h]) < 10:
                raw_pred = self.tcn_weight * tcn_raw + self.lgbm_weight * lgbm_raw
                # Convert from ATR-scaled label space to raw log-return space.
                # During warmup the raw blend is in mag units; multiply by the
                # current ATR ratio to get back to return space.
                result[f"pred_{h}d"] = float(raw_pred * atr_ratio)
                if not skip_buffer:
                    self._tcn_buffer[h].append(tcn_raw)
                    self._lgbm_buffer[h].append(lgbm_raw)
                continue

            # Z-score TCN prediction against its buffer
            tcn_z = self._zscore(tcn_raw, self._tcn_buffer[h])
            # Z-score LGBM prediction against its buffer
            lgbm_z = self._zscore(lgbm_raw, self._lgbm_buffer[h])

            # Blend z-scores, then denormalize to raw log-return using the
            # FIXED training-time label std (Deferred Fix 2) and the current
            # ATR ratio.  label_std is the std of the ATR-scaled labels (mag),
            # so we multiply by atr_ratio to return to raw return space.
            blend_z = self.tcn_weight * tcn_z + self.lgbm_weight * lgbm_z
            label_std = self._label_std.get(h, 0.012)
            raw_log_return = blend_z * label_std * atr_ratio

            result[f"pred_{h}d"] = float(raw_log_return)

            # Push raw predictions into buffers for future z-scoring
            if not skip_buffer:
                self._tcn_buffer[h].append(tcn_raw)
                self._lgbm_buffer[h].append(lgbm_raw)

        return result

    @staticmethod
    def _zscore(val: float, buf: deque[float]) -> float:
        """Z-score val against the buffer's mean and std. Returns 0.0 if buffer has < 2 elements."""
        if len(buf) < 2:
            return 0.0
        arr = np.array(buf, dtype=np.float64)
        mean = float(np.mean(arr))
        std = float(np.std(arr, ddof=1))
        if std < 1e-12:
            return 0.0
        return (val - mean) / std

    @staticmethod
    def _pooled_std(buf_a: deque[float], buf_b: deque[float]) -> float:
        """Proper pooled standard deviation: sqrt((s_a^2 + s_b^2) / 2).
        
        The previous implementation concatenated buffers, which included
        between-model mean offset as variance (incorrect). This formula
        pools the variances correctly.
        
        Returns 1.0 if insufficient data.
        """
        if len(buf_a) < 2 or len(buf_b) < 2:
            return 1.0
        arr_a = np.array(buf_a, dtype=np.float64)
        arr_b = np.array(buf_b, dtype=np.float64)
        std_a = float(np.std(arr_a, ddof=1))
        std_b = float(np.std(arr_b, ddof=1))
        # Proper pooled std: sqrt((s_a^2 + s_b^2) / 2)
        pooled = float(np.sqrt((std_a**2 + std_b**2) / 2))
        return pooled if pooled > 1e-12 else 1.0


# ── Request handling ──────────────────────────────────────────────────────────

def _handle_request(raw_bytes: bytes, ensembles: dict[str, EquityEnsemble], req_id: int) -> bytes:
    """Decode one V3 REQ message, run inference, return serialized reply.

    The request may include a ``symbol`` field (e.g. ``"QQQ"`` or ``"NVDA"``).
    When present, the corresponding per-symbol ensemble is used.  When absent
    (legacy clients), the first available ensemble is used as a fallback.
    """
    try:
        request: dict[str, Any] = json.loads(raw_bytes)
    except json.JSONDecodeError as exc:
        log.error("req_id=%d json_error=%s", req_id, str(exc))
        return json.dumps({"error": f"json decode error: {exc}"}).encode()

    # Select ensemble by symbol, fall back to the first available.
    symbol = request.get("symbol", "")
    ensemble = ensembles.get(symbol) or (next(iter(ensembles.values())) if ensembles else None)
    if ensemble is None:
        log.error("req_id=%d no_ensemble symbol=%s", req_id, symbol)
        return json.dumps({"error": f"no ensemble for symbol '{symbol}'"}).encode()

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

    # Detect healthcheck requests: seq_len=1 with all-zero features.
    # These must not pollute the z-score prediction buffers.
    is_healthcheck = (
        len(feature_window) == 1
        and all(abs(v) < 1e-12 for v in feature_window[0])
    )

    try:
        preds = ensemble.predict(feature_window, atr_ratio=atr_ratio, skip_buffer=is_healthcheck)
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
                "symbol": symbol,
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
    model_meta_path: str | None = None,
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

    return EquityEnsemble(
        tcn, lgbm_h1, lgbm_h5, lgbm_h21, tcn_weight, lgbm_weight, model_meta_path
    )


def run_service(
    zmq_bind: str,
    ensembles: dict[str, EquityEnsemble],
) -> int:
    """Start the ZMQ REP loop. Blocks until shutdown signal.

    Parameters
    ----------
    zmq_bind : str
        ZMQ bind address, e.g. ``tcp://*:5555``.
    ensembles : dict[str, EquityEnsemble]
        Per-symbol ensemble dict keyed by symbol (e.g. ``{"QQQ": ..., "NVDA": ...}``).
    """
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

    symbol_list = list(ensembles.keys())
    log.info("loaded ensembles for symbols: %s", symbol_list)

    # ── Bind ZMQ socket ───────────────────────────────────────────────────────
    context = zmq.Context()
    socket = context.socket(zmq.REP)
    try:
        socket.bind(zmq_bind)
    except zmq.ZMQError as exc:
        log.error("failed to bind ZMQ socket %s: %s", zmq_bind, exc)
        context.destroy()
        return 1

    log.info("ZMQ REP bound to %s — ready (V3 equities, %d symbols)", zmq_bind, len(symbol_list))

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
        reply = _handle_request(raw, ensembles, req_id)
        socket.send(reply)

    log.info("shutting down — processed %d requests", req_id)
    socket.close()
    context.term()
    return 0


def main() -> int:
    # ── Resolve paths from env ────────────────────────────────────────────────
    models_dir = Path(os.environ.get("MODELS_DIR", "models"))
    zmq_bind = os.environ.get("ZMQ_BIND", "tcp://*:5555")
    tcn_weight = float(os.environ.get("TCN_WEIGHT", "0.5"))
    lgbm_weight = float(os.environ.get("LGBM_WEIGHT", "0.5"))

    # ── Discover per-symbol model bundles ─────────────────────────────────
    # Each subdirectory under MODELS_DIR that contains a TCN checkpoint
    # (named *tcn*.pt) is treated as a symbol model bundle.  The directory
    # name is used as the symbol (e.g. "NVDA" → symbol="NVDA").
    # Legacy flat layout (models directly in MODELS_DIR) is treated as the
    # default symbol "QQQ" when a TCN is found there.
    ensembles: dict[str, EquityEnsemble] = {}

    # First, check for per-symbol subdirectories
    if models_dir.is_dir():
        for entry in sorted(models_dir.iterdir()):
            if not entry.is_dir():
                continue
            sym = entry.name
            tcn_files = list(entry.glob("*tcn*.pt"))
            if not tcn_files:
                continue
            tcn_path = str(tcn_files[0])
            lgbm_h1 = entry / f"{sym.lower()}_lgbm_h1_v1.pkl"
            lgbm_h5 = entry / f"{sym.lower()}_lgbm_h5_v1.pkl"
            lgbm_h21 = entry / f"{sym.lower()}_lgbm_h21_v1.pkl"
            if not lgbm_h1.exists():
                # Try lowercase prefix
                lgbm_h1 = entry / f"{sym.lower()}_lgbm_h1_v1.pkl"
            if not all(p.exists() for p in [lgbm_h1, lgbm_h5, lgbm_h21]):
                log.warning(
                    "skipping %s: missing LGBM files (%s, %s, %s)",
                    sym, lgbm_h1, lgbm_h5, lgbm_h21,
                )
                continue
            try:
                # Per-symbol model_meta sits next to the TCN checkpoint.
                meta_path = str(entry / f"model_meta_{sym.lower()}_v1.json")
                ensemble = _load_ensemble(
                    tcn_path, str(lgbm_h1), str(lgbm_h5), str(lgbm_h21),
                    tcn_weight, lgbm_weight, meta_path,
                )
                ensembles[sym] = ensemble
                log.info("loaded model bundle for symbol=%s tcn=%s", sym, tcn_path)
            except Exception as exc:
                log.error("failed to load ensemble for symbol=%s: %s", sym, exc)

    # Fallback: legacy flat layout (models directly in MODELS_DIR).
    # Always attempted — even when per-symbol subdirectories are found,
    # the flat layout provides the default QQQ ensemble.
    tcn_path = os.environ.get("TCN_PATH", str(models_dir / "qqq_tcn_v1.pt"))
    lgbm_h1_path = os.environ.get("LGBM_H1_PATH", str(models_dir / "qqq_lgbm_h1_v1.pkl"))
    lgbm_h5_path = os.environ.get("LGBM_H5_PATH", str(models_dir / "qqq_lgbm_h5_v1.pkl"))
    lgbm_h21_path = os.environ.get("LGBM_H21_PATH", str(models_dir / "qqq_lgbm_h21_v1.pkl"))

    if all(Path(p).exists() for p in [tcn_path, lgbm_h1_path, lgbm_h5_path, lgbm_h21_path]):
        qqq_meta = os.environ.get("MODEL_META_PATH", str(models_dir / "model_meta_qqq_v1.json"))
        try:
            ensemble = _load_ensemble(
                tcn_path, lgbm_h1_path, lgbm_h5_path, lgbm_h21_path,
                tcn_weight, lgbm_weight, qqq_meta,
            )
            ensembles["QQQ"] = ensemble
            log.info(
                "loaded default QQQ ensemble: tcn=%s lgbm=[%s,%s,%s]",
                tcn_path, lgbm_h1_path, lgbm_h5_path, lgbm_h21_path,
            )
        except Exception as exc:
            log.error("failed to load default QQQ ensemble: %s", exc)

    if not ensembles:
        print("error: no model bundles found in %s" % models_dir, file=sys.stderr)
        return 1

    log.info(
        "equity inference configured: %d ensembles, zmq_bind=%s, weights=(tcn=%.2f lgbm=%.2f)",
        len(ensembles), zmq_bind, tcn_weight, lgbm_weight,
    )

    return run_service(zmq_bind, ensembles)


if __name__ == "__main__":
    raise SystemExit(main())
